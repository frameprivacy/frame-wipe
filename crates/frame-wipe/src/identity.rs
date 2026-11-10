//! Stable device identity, captured at plan time and re-checked before the first write.

use crate::error::{Error, Guard};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Everything we use to notice a hot-plug or path reuse before the first write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub path: PathBuf,
    pub canonical_path: PathBuf,
    /// Name the operator must type, e.g. "sda", "disk2", "PhysicalDrive1", or a file name.
    pub display_name: String,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub wwn: Option<String>,
    pub size: u64,
    /// Unix (st_dev, st_ino) or Windows (volume serial, file index).
    pub file_id: Option<(u64, u64)>,
    pub media: String,
}

impl DeviceIdentity {
    pub fn summary(&self) -> String {
        let model = self.model.as_deref().unwrap_or("unknown model");
        let serial = self.serial.as_deref().unwrap_or("no serial");
        format!(
            "{name}  {model}  {serial}  {size} bytes  {path}",
            name = self.display_name,
            size = self.size,
            path = self.canonical_path.display()
        )
    }

    /// A stable identity is a serial, a WWN, or a regular-file inode/file-id.
    pub fn is_identified(&self) -> bool {
        self.serial
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
            || self.wwn.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
            || self.file_id.is_some()
    }

    /// Compare against a freshly captured identity. Path, serial, size and file id must all match
    /// when they are present on either side.
    pub fn mismatch(&self, fresh: &DeviceIdentity) -> Option<String> {
        let mut parts = Vec::new();
        if self.display_name != fresh.display_name {
            parts.push(format!(
                "name {} -> {}",
                self.display_name, fresh.display_name
            ));
        }
        if paths_differ(&self.canonical_path, &fresh.canonical_path)
            && paths_differ(&self.path, &fresh.path)
        {
            parts.push(format!(
                "path {} -> {}",
                self.canonical_path.display(),
                fresh.canonical_path.display()
            ));
        }
        if let (Some(a), Some(b)) = (self.serial.as_ref(), fresh.serial.as_ref()) {
            if a != b {
                parts.push(format!("serial {a} -> {b}"));
            }
        } else if self.serial != fresh.serial {
            parts.push(format!("serial {:?} -> {:?}", self.serial, fresh.serial));
        }
        if self.size != fresh.size {
            parts.push(format!("size {} -> {}", self.size, fresh.size));
        }
        if let (Some(a), Some(b)) = (self.file_id, fresh.file_id) {
            if a != b {
                parts.push(format!("file id {a:?} -> {b:?}"));
            }
        }
        if let (Some(a), Some(b)) = (self.wwn.as_ref(), fresh.wwn.as_ref()) {
            if a != b {
                parts.push(format!("wwn {a} -> {b}"));
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        }
    }
}

fn paths_differ(a: &Path, b: &Path) -> bool {
    if a == b {
        return false;
    }
    // Windows paths are compared case-insensitively.
    if cfg!(windows) {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
            == false
    } else {
        true
    }
}

/// Abort if a planned identity does not match a freshly captured one.
pub fn assert_same(planned: &DeviceIdentity, fresh: &DeviceIdentity) -> Result<(), Error> {
    match planned.mismatch(fresh) {
        None => Ok(()),
        Some(detail) => Err(Error::IdentityMismatch {
            expected: planned.summary(),
            found: format!("{} ({detail})", fresh.summary()),
        }),
    }
}

pub fn missing_identity_error(id: &DeviceIdentity) -> Error {
    Error::refused(
        Guard::MissingIdentity,
        format!(
            "no serial, WWN or file id for {} at {}; refusing to wipe an unidentified device (pass --allow-unidentified if you accept path-only identification)",
            id.display_name,
            id.path.display()
        ),
    )
}

/// Confirmation must match the device name (display_name). Comparison is
/// case-insensitive on Windows, case-sensitive elsewhere. Leading `/dev/` or `\\.\\` is ignored.
pub fn confirmation_matches(display_name: &str, typed: &str) -> bool {
    let want = normalize_name(display_name);
    let got = normalize_name(typed);
    if want.is_empty() || got.is_empty() {
        return false;
    }
    if cfg!(windows) {
        want.eq_ignore_ascii_case(&got)
    } else {
        want == got
    }
}

fn normalize_name(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix(r"\\?\").unwrap_or(s);
    let s = s.strip_prefix(r"\\.\").unwrap_or(s);
    let s = s
        .strip_prefix("/dev/r")
        .unwrap_or(s.strip_prefix("/dev/").unwrap_or(s));
    s.trim_end_matches('/').to_string()
}
