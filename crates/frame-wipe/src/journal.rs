//! Power-loss-safe progress journal. Each checkpoint is written, flushed, and then
//! the file is `sync_all`'d so an incomplete wipe can be reported after a crash.

use crate::error::Error;
use crate::identity::DeviceIdentity;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const JOURNAL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Journal {
    pub version: u32,
    pub identity: DeviceIdentity,
    pub started_utc: String,
    pub method: String,
    pub pass: u32,
    pub passes: u32,
    pub offset: u64,
    pub size: u64,
    pub phase: String,
    pub complete: bool,
}

impl Journal {
    pub fn new(
        identity: &DeviceIdentity,
        method: &str,
        passes: u32,
        size: u64,
        started_utc: &str,
    ) -> Self {
        Self {
            version: JOURNAL_VERSION,
            identity: identity.clone(),
            started_utc: started_utc.to_string(),
            method: method.to_string(),
            pass: 1,
            passes,
            offset: 0,
            size,
            phase: "starting".into(),
            complete: false,
        }
    }
}

pub fn journal_path_for(target: &Path) -> PathBuf {
    let file_name = target
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "target".into());
    let mut p = target.to_path_buf();
    p.set_file_name(format!(".{file_name}.frame-wipe-journal"));
    p
}

pub fn read(path: &Path) -> Result<Option<Journal>, Error> {
    if !path.exists() {
        return Ok(None);
    }
    let data =
        fs::read(path).map_err(|e| Error::Journal(format!("read {}: {e}", path.display())))?;
    if data.is_empty() {
        return Ok(None);
    }
    let j: Journal = serde_json::from_slice(&data)
        .map_err(|e| Error::Journal(format!("parse {}: {e}", path.display())))?;
    if j.version != JOURNAL_VERSION {
        return Err(Error::Journal(format!(
            "unsupported journal version {} in {}",
            j.version,
            path.display()
        )));
    }
    Ok(Some(j))
}

/// Write a checkpoint. The contents are written to a sibling temp file, flushed,
/// and then renamed over the journal path (atomic on the same volume on Unix).
pub fn checkpoint(path: &Path, journal: &Journal) -> Result<(), Error> {
    let tmp = path.with_extension("journal.tmp");
    let data = serde_json::to_vec_pretty(journal)
        .map_err(|e| Error::Journal(format!("serialize: {e}")))?;
    {
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)
            .map_err(|e| Error::Journal(format!("open {}: {e}", tmp.display())))?;
        f.write_all(&data)
            .map_err(|e| Error::Journal(format!("write {}: {e}", tmp.display())))?;
        f.flush()
            .map_err(|e| Error::Journal(format!("flush {}: {e}", tmp.display())))?;
        f.sync_all()
            .map_err(|e| Error::Journal(format!("sync {}: {e}", tmp.display())))?;
    }
    fs::rename(&tmp, path).map_err(|e| Error::Journal(format!("rename journal: {e}")))?;
    // Best-effort parent dir sync so the rename itself survives power loss.
    if let Some(dir) = path.parent() {
        if let Ok(dirf) = File::open(dir) {
            let _ = dirf.sync_all();
        }
    }
    Ok(())
}

pub fn complete_and_remove(path: &Path, journal: &mut Journal) -> Result<(), Error> {
    journal.complete = true;
    journal.phase = "done".into();
    journal.offset = journal.size;
    checkpoint(path, journal)?;
    fs::remove_file(path).map_err(|e| Error::Journal(format!("remove {}: {e}", path.display())))?;
    Ok(())
}

pub fn load_for_resume(path: &Path, expected: &DeviceIdentity) -> Result<Option<Journal>, Error> {
    match read(path)? {
        None => Ok(None),
        Some(j) => {
            if let Some(detail) = j.identity.mismatch(expected) {
                return Err(Error::Journal(format!(
                    "existing journal does not match the current device identity ({detail}); refusing to resume. Delete {} only if you know the previous run is finished.",
                    path.display()
                )));
            }
            Ok(Some(j))
        }
    }
}
