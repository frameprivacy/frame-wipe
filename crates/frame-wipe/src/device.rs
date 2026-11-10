//! Device enumeration and path inspection. Platform details live in `device::*`.

use crate::error::Error;
use crate::identity::DeviceIdentity;
use crate::planner::{MediaKind, TargetSnapshot};
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
mod unix_meta;
#[cfg(windows)]
pub(crate) mod win_meta;

#[derive(Debug, Clone, Copy, Default)]
pub struct EnumerateOptions {
    /// Include loop, ram, and other virtual devices.
    pub include_virtual: bool,
}

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub snapshot: TargetSnapshot,
}

impl DeviceInfo {
    pub fn identity(&self) -> &DeviceIdentity {
        &self.snapshot.identity
    }
}

pub fn enumerate(opts: EnumerateOptions) -> Result<Vec<DeviceInfo>, Error> {
    #[cfg(target_os = "linux")]
    {
        return linux::enumerate(opts);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::enumerate(opts);
    }
    #[cfg(windows)]
    {
        return windows::enumerate(opts);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = opts;
        Err(Error::Message(
            "device enumeration is not supported on this platform".into(),
        ))
    }
}

/// Inspect a user-supplied path (device node, volume, or regular file / disk image).
/// Never follows a symlink: a symlink is reported with `is_symlink = true`.
pub fn inspect(path: &Path) -> Result<TargetSnapshot, Error> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| Error::Open {
        path: path.to_path_buf(),
        detail: e.to_string(),
    })?;
    if meta.file_type().is_symlink() {
        return Ok(symlink_snapshot(path, &meta));
    }
    if meta.file_type().is_file() {
        return inspect_regular_file(path, &meta);
    }
    if meta.file_type().is_dir() {
        return inspect_directory(path);
    }
    // Block / character devices and volumes go through the platform inspector.
    #[cfg(target_os = "linux")]
    {
        return linux::inspect_device(path);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::inspect_device(path);
    }
    #[cfg(windows)]
    {
        return windows::inspect_device(path);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        Err(Error::InvalidTarget(path.to_path_buf()))
    }
}

/// Recapture identity for the identity-recheck guard. Never follows symlinks.
pub fn recapture(path: &Path) -> Result<DeviceIdentity, Error> {
    Ok(inspect(path)?.identity)
}

fn symlink_snapshot(path: &Path, _meta: &std::fs::Metadata) -> TargetSnapshot {
    let name = file_display_name(path);
    TargetSnapshot {
        identity: DeviceIdentity {
            path: path.to_path_buf(),
            canonical_path: path.to_path_buf(),
            display_name: name,
            model: None,
            serial: None,
            wwn: None,
            size: 0,
            file_id: None,
            media: "symlink".into(),
        },
        kind: MediaKind::Unknown,
        is_system: false,
        is_app_disk: false,
        mounts: Vec::new(),
        in_use: false,
        is_raid_member: false,
        is_lvm_member: false,
        backing_file: None,
        is_symlink: true,
        is_regular_file: false,
        is_directory: false,
        is_block_device: false,
        logical_block_size: 512,
    }
}

fn inspect_regular_file(path: &Path, meta: &std::fs::Metadata) -> Result<TargetSnapshot, Error> {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let file_id = file_id_of(path, meta);
    let name = file_display_name(path);
    Ok(TargetSnapshot {
        identity: DeviceIdentity {
            path: path.to_path_buf(),
            canonical_path: canon,
            display_name: name,
            model: Some("disk image".into()),
            serial: file_id.map(|(d, i)| format!("file-{d:x}-{i:x}")),
            wwn: None,
            size: meta.len(),
            file_id,
            media: "image".into(),
        },
        kind: MediaKind::Image,
        is_system: false,
        is_app_disk: crate::safety::is_current_exe(path),
        mounts: Vec::new(),
        in_use: false,
        is_raid_member: false,
        is_lvm_member: false,
        backing_file: Some(path.to_path_buf()),
        is_symlink: false,
        is_regular_file: true,
        is_directory: false,
        is_block_device: false,
        logical_block_size: 4096,
    })
}

fn inspect_directory(path: &Path) -> Result<TargetSnapshot, Error> {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let name = file_display_name(path);
    Ok(TargetSnapshot {
        identity: DeviceIdentity {
            path: path.to_path_buf(),
            canonical_path: canon,
            display_name: name,
            model: None,
            serial: None,
            wwn: None,
            size: 0,
            file_id: file_id_of(path, &std::fs::symlink_metadata(path)?),
            media: "directory".into(),
        },
        kind: MediaKind::Unknown,
        is_system: false,
        is_app_disk: false,
        mounts: vec![path.display().to_string()],
        in_use: false,
        is_raid_member: false,
        is_lvm_member: false,
        backing_file: None,
        is_symlink: false,
        is_regular_file: false,
        is_directory: true,
        is_block_device: false,
        logical_block_size: 4096,
    })
}

pub(crate) fn file_display_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn file_id_of(path: &Path, meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        let _ = path;
        return unix_meta::file_id(meta);
    }
    #[cfg(windows)]
    {
        return win_meta::file_id(path, meta);
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (path, meta);
        None
    }
}

/// Parent disk the current executable lives on, as a platform path prefix or name.
pub fn app_disk_names() -> Vec<String> {
    let Ok(exe) = std::env::current_exe() else {
        return Vec::new();
    };
    #[cfg(target_os = "linux")]
    {
        return linux::disks_for_path(&exe);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::disks_for_path(&exe);
    }
    #[cfg(windows)]
    {
        return windows::disks_for_path(&exe);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = exe;
        Vec::new()
    }
}

pub fn is_elevated() -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(windows)]
    {
        windows::is_elevated()
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

pub fn elevation_hint() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "re-run with sudo or pkexec"
    }
    #[cfg(target_os = "macos")]
    {
        "re-run with sudo, or grant administrator rights"
    }
    #[cfg(windows)]
    {
        "re-run this program as Administrator"
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        "re-run with elevated privileges"
    }
}

pub fn raw_path_for(display_or_path: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        macos::prefer_rdisk(display_or_path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        display_or_path.to_path_buf()
    }
}
