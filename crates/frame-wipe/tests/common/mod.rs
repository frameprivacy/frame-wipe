#![allow(dead_code)]
use frame_wipe::identity::DeviceIdentity;
use frame_wipe::planner::{MediaKind, TargetSnapshot};
use std::path::PathBuf;

pub fn image_snapshot(name: &str, size: u64) -> TargetSnapshot {
    TargetSnapshot {
        identity: DeviceIdentity {
            path: PathBuf::from(format!("/tmp/{name}")),
            canonical_path: PathBuf::from(format!("/tmp/{name}")),
            display_name: name.to_string(),
            model: Some("disk image".into()),
            serial: Some(format!("file-1-{name}")),
            wwn: None,
            size,
            file_id: Some((1, 2)),
            media: "image".into(),
        },
        kind: MediaKind::Image,
        is_system: false,
        is_app_disk: false,
        mounts: Vec::new(),
        in_use: false,
        is_raid_member: false,
        is_lvm_member: false,
        backing_file: Some(PathBuf::from(format!("/tmp/{name}"))),
        is_symlink: false,
        is_regular_file: true,
        is_directory: false,
        is_block_device: false,
        logical_block_size: 4096,
    }
}

pub fn device_snapshot(name: &str, size: u64) -> TargetSnapshot {
    let mut s = image_snapshot(name, size);
    s.identity.path = PathBuf::from(format!("/dev/{name}"));
    s.identity.canonical_path = s.identity.path.clone();
    s.identity.model = Some("TEST DISK".into());
    s.identity.serial = Some("SERIAL123".into());
    s.identity.file_id = Some((8, 100));
    s.kind = MediaKind::Hdd;
    s.is_regular_file = false;
    s.is_block_device = true;
    s.backing_file = None;
    s
}
