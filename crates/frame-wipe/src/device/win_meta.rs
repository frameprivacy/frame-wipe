#![cfg(windows)]
use std::os::windows::fs::MetadataExt;
use std::path::Path;

pub fn file_id(path: &Path, meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    // volume serial + file index, when the std metadata exposes them.
    let vol = meta.volume_serial_number()?;
    let idx = meta.file_index()?;
    let _ = path;
    Some((vol as u64, idx))
}
