#![cfg(windows)]
use std::fs::OpenOptions;
use std::mem::zeroed;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

/// Volume serial and file index via GetFileInformationByHandle (stable).
pub fn file_id(path: &Path, _meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    let info = by_handle_info(path)?;
    let idx = ((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64);
    Some((info.dwVolumeSerialNumber as u64, idx))
}

pub fn volume_serial(path: &Path) -> Option<u64> {
    Some(by_handle_info(path)?.dwVolumeSerialNumber as u64)
}

fn by_handle_info(path: &Path) -> Option<BY_HANDLE_FILE_INFORMATION> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(path)
        .ok()?;
    unsafe {
        let mut info: BY_HANDLE_FILE_INFORMATION = zeroed();
        if GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) == 0 {
            return None;
        }
        Some(info)
    }
}
