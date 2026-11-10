#![cfg(unix)]
use std::os::unix::fs::MetadataExt;

pub fn file_id(meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    Some((meta.dev(), meta.ino()))
}
