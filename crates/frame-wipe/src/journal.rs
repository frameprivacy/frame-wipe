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
    /// CSPRNG seed for a random pass, so --resume can rewrite and verify
    /// the same stream. Hex is avoided; serde stores this as a byte array.
    #[serde(default)]
    pub random_seed: Option<[u8; 32]>,
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
            random_seed: None,
        }
    }
}

/// Conservative NAME_MAX used so journal sidecars and their `.journal.tmp`
/// siblings stay creatable on common filesystems.
const MAX_FILE_NAME_BYTES: usize = 255;

pub fn journal_path_for(target: &Path) -> PathBuf {
    if parent_is_device_namespace(target) {
        let digest = fnv1a64(target.to_string_lossy().as_bytes());
        return journal_cache_dir().join(format!(".frame-wipe-{digest:016x}.journal"));
    }
    let file_name = target
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "target".into());
    let mut p = target.to_path_buf();
    let pretty = format!(".{file_name}.frame-wipe-journal");
    if pretty.len() <= MAX_FILE_NAME_BYTES {
        p.set_file_name(pretty);
        return p;
    }
    // A long target name would push the sidecar (or its temp rename) past
    // NAME_MAX. Use a short, stable hash of the path instead.
    let digest = fnv1a64(target.to_string_lossy().as_bytes());
    p.set_file_name(format!(".frame-wipe-{digest:016x}.journal"));
    p
}

fn parent_is_device_namespace(target: &Path) -> bool {
    let s = target.to_string_lossy();
    let lower = s.replace('\\', "/").to_ascii_lowercase();
    if lower == "/dev" || lower.starts_with("/dev/") {
        return true;
    }
    crate::safety::path_looks_like_raw_device(target)
        || crate::safety::windows_path_looks_like_raw_device(&s)
}

fn journal_cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("FRAME_WIPE_JOURNAL_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("frame-wipe");
        }
    }
    if cfg!(windows) {
        if let Ok(base) = std::env::var("LOCALAPPDATA") {
            if !base.is_empty() {
                return PathBuf::from(base).join("frame-wipe");
            }
        }
    }
    if cfg!(target_os = "macos") {
        if let Ok(home) = std::env::var("HOME") {
            if !home.is_empty() {
                return PathBuf::from(home).join("Library/Caches/frame-wipe");
            }
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home).join(".cache/frame-wipe");
        }
    }
    std::env::temp_dir().join("frame-wipe")
}

fn fnv1a64(data: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET;
    for b in data {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
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
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            Error::Journal(format!(
                "create journal directory {}: {e}",
                parent.display()
            ))
        })?;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn short_names_keep_a_readable_sidecar() {
        let p = journal_path_for(Path::new("/tmp/doc.txt"));
        assert_eq!(p, PathBuf::from("/tmp/.doc.txt.frame-wipe-journal"));
    }

    #[test]
    fn long_unicode_names_fit_name_max() {
        let long = format!("wiped-{}-end.bin", "あ".repeat(80));
        let target = PathBuf::from("/tmp").join(&long);
        assert!(
            long.len() + ".x".len() <= 255 || long.len() > 200,
            "test fixture should be a long utf-8 name, got {} bytes",
            long.len()
        );
        let pretty = format!(".{long}.frame-wipe-journal");
        assert!(
            pretty.len() > MAX_FILE_NAME_BYTES,
            "fixture should overflow NAME_MAX, got {}",
            pretty.len()
        );
        let journal = journal_path_for(&target);
        let name = journal.file_name().unwrap().to_string_lossy();
        assert!(
            name.len() <= MAX_FILE_NAME_BYTES,
            "journal name too long: {name} ({} bytes)",
            name.len()
        );
        let tmp = journal.with_extension("journal.tmp");
        let tmp_name = tmp.file_name().unwrap().to_string_lossy();
        assert!(
            tmp_name.len() <= MAX_FILE_NAME_BYTES,
            "journal temp name too long: {tmp_name} ({} bytes)",
            tmp_name.len()
        );
        assert_eq!(journal_path_for(&target), journal);
        assert!(name.starts_with(".frame-wipe-") && name.ends_with(".journal"));
    }

    #[test]
    fn raw_device_journals_are_not_written_under_dev() {
        fn looks_like_device_namespace(path: &Path) -> bool {
            let lower = path
                .to_string_lossy()
                .replace("\\", "/")
                .to_ascii_lowercase();
            lower.contains("/dev/")
                || lower.contains("//./")
                || lower.contains("//?/")
                || lower.starts_with("//.")
        }

        for target in [
            Path::new("/dev/sdb"),
            Path::new("/dev/nvme0n1"),
            Path::new("/dev/disk2"),
            Path::new("/dev/loop0"),
            Path::new(r"\\.\PhysicalDrive1"),
        ] {
            let journal = journal_path_for(target);
            assert!(
                !looks_like_device_namespace(&journal),
                "journal for {} must not live in a device namespace: {}",
                target.display(),
                journal.display()
            );
        }
    }
}
