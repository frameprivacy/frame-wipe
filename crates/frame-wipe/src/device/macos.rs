//! macOS enumeration via `diskutil` when present, with a /dev fallback.

use crate::device::{DeviceInfo, EnumerateOptions};
use crate::error::Error;
use crate::identity::DeviceIdentity;
use crate::planner::TargetSnapshot;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn prefer_rdisk(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("/dev/disk") {
        return PathBuf::from(format!("/dev/rdisk{rest}"));
    }
    if s.starts_with("/dev/rdisk") {
        return path.to_path_buf();
    }
    path.to_path_buf()
}

pub fn enumerate(opts: EnumerateOptions) -> Result<Vec<DeviceInfo>, Error> {
    if let Ok(list) = diskutil_list() {
        let mut out = Vec::new();
        let system = disks_for_path(Path::new("/"));
        let app = disks_for_path(&std::env::current_exe().unwrap_or_default());
        for id in list {
            if !opts.include_virtual && is_virtual_id(&id) {
                continue;
            }
            match inspect_named(&id, &system, &app) {
                Ok(snap) => out.push(DeviceInfo { snapshot: snap }),
                Err(_) => continue,
            }
        }
        return Ok(out);
    }
    enumerate_dev(opts)
}

pub fn inspect_device(path: &Path) -> Result<TargetSnapshot, Error> {
    let id = disk_id(path);
    let system = disks_for_path(Path::new("/"));
    let app = disks_for_path(&std::env::current_exe().unwrap_or_default());
    inspect_named(&id, &system, &app)
}

pub fn disks_for_path(path: &Path) -> Vec<String> {
    if let Ok(info) = diskutil_info(path) {
        let mut v = Vec::new();
        if let Some(whole) = info.whole_disk.map(normalize_id) {
            v.push(whole);
        }
        if let Some(id) = info.device_identifier.map(normalize_id) {
            if !v.contains(&id) {
                v.push(id);
            }
        }
        return v;
    }
    Vec::new()
}

fn disk_id(path: &Path) -> String {
    let s = path.to_string_lossy();
    let s = s
        .strip_prefix("/dev/r")
        .or_else(|| s.strip_prefix("/dev/"))
        .unwrap_or(&s);
    s.trim().to_string()
}

fn normalize_id(s: String) -> String {
    let s = s.trim();
    s.strip_prefix("/dev/r")
        .or_else(|| s.strip_prefix("/dev/"))
        .unwrap_or(s)
        .to_string()
}

fn is_virtual_id(id: &str) -> bool {
    id.contains("disk image") // not used this way
        || id.starts_with("loop")
}

fn inspect_named(id: &str, system: &[String], app: &[String]) -> Result<TargetSnapshot, Error> {
    let info = diskutil_info(&PathBuf::from(format!("/dev/{id}"))).ok();
    let size = info
        .as_ref()
        .and_then(|i| i.size_bytes)
        .or_else(|| dev_size(&PathBuf::from(format!("/dev/{id}"))))
        .unwrap_or(0);
    let model = info
        .as_ref()
        .and_then(|i| i.media_name.clone().or(i.device_node.clone()));
    let serial = info
        .as_ref()
        .and_then(|i| i.io_registry_entry_name.clone())
        .or_else(|| info.as_ref().and_then(|i| i.volume_uuid.clone()));
    // Prefer explicit serial from `diskutil info -plist` if we parsed it.
    let serial = info.as_ref().and_then(|i| i.serial.clone()).or(serial);
    let solid = info.as_ref().and_then(|i| i.solid_state).unwrap_or(false);
    let protocol = info
        .as_ref()
        .and_then(|i| i.protocol.clone())
        .unwrap_or_default();
    let virtual_media = info
        .as_ref()
        .and_then(|i| i.virtual_or_physical.clone())
        .map(|s| s.eq_ignore_ascii_case("Virtual"))
        .unwrap_or(false)
        || info
            .as_ref()
            .and_then(|i| i.media_type.clone())
            .map(|s| s.to_ascii_lowercase().contains("disk image"))
            .unwrap_or(false);
    let kind = super::kind::from_macos_signals(&protocol, solid, virtual_media, id, info.is_some());
    let mounts = info
        .as_ref()
        .and_then(|i| i.mount_point.clone())
        .map(|m| vec![m])
        .unwrap_or_default();
    let raid = info
        .as_ref()
        .and_then(|i| i.raid_or_csm.as_ref())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
        || info
            .as_ref()
            .and_then(|i| i.content.clone())
            .map(|c| {
                let c = c.to_ascii_lowercase();
                c.contains("apple_raid")
                    || c.contains("coredstorage")
                    || c.contains("apple_corestorage")
            })
            .unwrap_or(false);
    let whole = info
        .as_ref()
        .and_then(|i| i.whole_disk.clone())
        .map(normalize_id);
    let is_system = system.iter().any(|s| s == id || Some(s) == whole.as_ref());
    let is_app = app.iter().any(|s| s == id || Some(s) == whole.as_ref());
    let path = PathBuf::from(format!("/dev/{id}"));
    let raw = prefer_rdisk(&path);
    let lba = info
        .as_ref()
        .and_then(|i| i.device_block_size)
        .unwrap_or(4096);
    let backing = if virtual_media {
        info.as_ref()
            .and_then(|i| i.image_path.clone())
            .map(PathBuf::from)
    } else {
        None
    };

    Ok(TargetSnapshot {
        identity: DeviceIdentity {
            path: raw,
            canonical_path: path,
            display_name: id.to_string(),
            model,
            serial,
            wwn: info.as_ref().and_then(|i| i.disk_uuid.clone()),
            size,
            file_id: None,
            media: format!("{:?}", kind).to_ascii_lowercase(),
        },
        kind,
        is_system,
        is_app_disk: is_app,
        mounts: mounts.clone(),
        in_use: !mounts.is_empty(),
        is_raid_member: raid,
        is_lvm_member: false,
        backing_file: backing,
        is_symlink: false,
        is_regular_file: false,
        is_directory: false,
        is_block_device: true,
        logical_block_size: if lba == 0 { 512 } else { lba },
    })
}

#[derive(Debug, Default, Deserialize)]
struct DiskutilInfo {
    #[serde(rename = "DeviceIdentifier")]
    device_identifier: Option<String>,
    #[serde(rename = "DeviceNode")]
    device_node: Option<String>,
    #[serde(rename = "MediaName")]
    media_name: Option<String>,
    #[serde(rename = "IORegistryEntryName")]
    io_registry_entry_name: Option<String>,
    #[serde(rename = "VolumeUUID")]
    volume_uuid: Option<String>,
    #[serde(rename = "DiskUUID")]
    disk_uuid: Option<String>,
    #[serde(
        rename = "SerialNumber",
        alias = "DeviceSerialNumber",
        alias = "IOSerialNumber"
    )]
    serial: Option<String>,
    #[serde(rename = "TotalSize", alias = "Size")]
    size_bytes: Option<u64>,
    #[serde(rename = "SolidState")]
    solid_state: Option<bool>,
    #[serde(rename = "Protocol")]
    protocol: Option<String>,
    #[serde(rename = "VirtualOrPhysical")]
    virtual_or_physical: Option<String>,
    #[serde(rename = "MediaType")]
    media_type: Option<String>,
    #[serde(rename = "MountPoint")]
    mount_point: Option<String>,
    #[serde(rename = "Content")]
    content: Option<String>,
    #[serde(rename = "RAIDSetUUID", alias = "CoreStorageUUID")]
    raid_or_csm: Option<String>,
    #[serde(rename = "ParentWholeDisk", alias = "WholeDisk")]
    whole_disk: Option<String>,
    #[serde(rename = "DeviceBlockSize")]
    device_block_size: Option<u64>,
    #[serde(rename = "ImagePath", alias = "DiskImagePath")]
    image_path: Option<String>,
}

fn diskutil_info(path: &Path) -> Result<DiskutilInfo, Error> {
    let out = Command::new("diskutil")
        .args(["info", "-plist"])
        .arg(path)
        .output()
        .map_err(|e| Error::Message(format!("diskutil info failed: {e}")))?;
    if !out.status.success() {
        return Err(Error::Message(format!(
            "diskutil info {} exited {}",
            path.display(),
            out.status
        )));
    }
    plist::from_bytes(&out.stdout).map_err(|e| Error::Message(format!("parse diskutil plist: {e}")))
}

fn diskutil_list() -> Result<Vec<String>, Error> {
    let out = Command::new("diskutil")
        .args(["list", "-plist"])
        .output()
        .map_err(|e| Error::Message(format!("diskutil list failed: {e}")))?;
    if !out.status.success() {
        return Err(Error::Message("diskutil list failed".into()));
    }
    let val: plist::Value = plist::from_bytes(&out.stdout)
        .map_err(|e| Error::Message(format!("parse diskutil list: {e}")))?;
    let mut ids = Vec::new();
    if let Some(arr) = val
        .as_dictionary()
        .and_then(|d| d.get("AllDisks"))
        .and_then(|v| v.as_array())
    {
        for v in arr {
            if let Some(s) = v.as_string() {
                ids.push(s.to_string());
            }
        }
    }
    if ids.is_empty() {
        return Err(Error::Message("diskutil list returned no disks".into()));
    }
    Ok(ids)
}

fn enumerate_dev(opts: EnumerateOptions) -> Result<Vec<DeviceInfo>, Error> {
    let mut out = Vec::new();
    let rd = match fs::read_dir("/dev") {
        Ok(v) => v,
        Err(e) => return Err(Error::from(e)),
    };
    let system = disks_for_path(Path::new("/"));
    let app = disks_for_path(&std::env::current_exe().unwrap_or_default());
    for ent in rd.flatten() {
        let name = ent.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("disk") {
            continue;
        }
        if name.starts_with("rdisk") {
            continue;
        }
        let _ = opts;
        if let Ok(snap) = inspect_named(&name, &system, &app) {
            out.push(DeviceInfo { snapshot: snap });
        }
    }
    Ok(out)
}

fn dev_size(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).ok().map(|m| m.blocks() * 512)
}
