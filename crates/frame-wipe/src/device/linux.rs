//! Linux device enumeration from sysfs and /proc. Never opens a device for write.

use crate::device::{DeviceInfo, EnumerateOptions};
use crate::error::Error;
use crate::identity::DeviceIdentity;
use crate::planner::{MediaKind, TargetSnapshot};
use std::fs;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};

pub fn enumerate(opts: EnumerateOptions) -> Result<Vec<DeviceInfo>, Error> {
    let mut out = Vec::new();
    let block = Path::new("/sys/block");
    let entries = match fs::read_dir(block) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(Error::from(e)),
    };
    let system = system_disk_names();
    let app = disks_for_path_inner(&std::env::current_exe().unwrap_or_default());
    let mounts = read_mounts();
    let swaps = read_swaps();
    for ent in entries {
        let ent = match ent {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = ent.file_name();
        let name = name.to_string_lossy();
        if skip_name(&name, opts.include_virtual) {
            continue;
        }
        match inspect_named(&name, &system, &app, &mounts, &swaps) {
            Ok(snap) => out.push(DeviceInfo { snapshot: snap }),
            Err(_) => continue,
        }
        // Partitions
        if let Ok(parts) = fs::read_dir(ent.path()) {
            for p in parts.flatten() {
                let pn = p.file_name();
                let pn = pn.to_string_lossy();
                if !is_partition_dir(&name, &pn) {
                    continue;
                }
                if let Ok(snap) = inspect_named(&pn, &system, &app, &mounts, &swaps) {
                    out.push(DeviceInfo { snapshot: snap });
                }
            }
        }
    }
    out.sort_by(|a, b| {
        a.snapshot
            .identity
            .display_name
            .cmp(&b.snapshot.identity.display_name)
    });
    Ok(out)
}

pub fn inspect_device(path: &Path) -> Result<TargetSnapshot, Error> {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = name.trim_start_matches('r'); // /dev/r* does not exist on Linux, harmless
    let system = system_disk_names();
    let app = disks_for_path_inner(&std::env::current_exe().unwrap_or_default());
    let mounts = read_mounts();
    let swaps = read_swaps();
    inspect_named(&name, &system, &app, &mounts, &swaps)
}

pub fn disks_for_path(path: &Path) -> Vec<String> {
    disks_for_path_inner(path)
}

fn disks_for_path_inner(path: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let mounts = read_mounts();
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut best: Option<(usize, String)> = None;
    for m in &mounts {
        if canon.starts_with(&m.mountpoint) {
            let len = m.mountpoint.as_os_str().len();
            if best.as_ref().map(|(l, _)| len > *l).unwrap_or(true) {
                best = Some((len, m.source.clone()));
            }
        }
    }
    if let Some((_, source)) = best {
        if let Some(n) = device_name_from_source(&source) {
            names.push(n.clone());
            if let Some(parent) = parent_disk(&n) {
                if parent != n {
                    names.push(parent);
                }
            }
        }
    }
    names
}

fn skip_name(name: &str, include_virtual: bool) -> bool {
    if name.starts_with("loop")
        || name.starts_with("ram")
        || name.starts_with("zram")
        || name == "sr0"
    {
        return !include_virtual;
    }
    // Skip floppy and cd typically unless include_virtual (optical still listed if include)
    if name.starts_with("fd") {
        return true;
    }
    false
}

fn is_partition_dir(disk: &str, child: &str) -> bool {
    if !child.starts_with(disk) {
        return false;
    }
    Path::new("/sys/block")
        .join(disk)
        .join(child)
        .join("partition")
        .exists()
        || Path::new("/sys/class/block")
            .join(child)
            .join("partition")
            .exists()
}

fn inspect_named(
    name: &str,
    system: &[String],
    app: &[String],
    mounts: &[Mount],
    swaps: &[String],
) -> Result<TargetSnapshot, Error> {
    let sys = class_block(name);
    let size = read_u64(&sys.join("size")).unwrap_or(0).saturating_mul(512);
    let model = read_trim(&sys.join("device/model"))
        .or_else(|| read_trim(&sys.join("device/name")))
        .or_else(|| nvme_model(name));
    let serial = read_trim(&sys.join("device/serial"))
        .or_else(|| read_trim(&sys.join("device/wwid")))
        .or_else(|| nvme_serial(name));
    let wwn = read_trim(&sys.join("device/wwid")).or_else(|| read_trim(&sys.join("wwid")));
    let rotational = read_trim(&sys.join("queue/rotational"));
    let block = read_u64(&sys.join("queue/logical_block_size")).unwrap_or(512);
    let backing = read_trim(&sys.join("loop/backing_file")).map(PathBuf::from);
    let holders = list_names(&sys.join("holders"));
    let is_raid = name.starts_with("md")
        || sys.join("md").exists()
        || holders.iter().any(|h| h.starts_with("md"))
        || is_md_member(name);
    let is_lvm = name.starts_with("dm-")
        || sys.join("dm").exists()
        || holders.iter().any(|h| h.starts_with("dm-"));
    let kind = if name.starts_with("loop") {
        MediaKind::Loop
    } else if name.starts_with("ram") || name.starts_with("zram") {
        MediaKind::Ram
    } else if backing.is_some() {
        MediaKind::Virtual
    } else if name.starts_with("nvme") {
        MediaKind::Nvme
    } else if name.starts_with("sr") {
        MediaKind::Optical
    } else if rotational.as_deref() == Some("0") {
        MediaKind::Ssd
    } else if rotational.as_deref() == Some("1") {
        MediaKind::Hdd
    } else {
        MediaKind::Unknown
    };
    let dev_path = PathBuf::from(format!("/dev/{name}"));
    let our_mounts: Vec<String> = mounts
        .iter()
        .filter(|m| source_matches_device(&m.source, name))
        .map(|m| m.mountpoint.display().to_string())
        .collect();
    let in_swap = swaps.iter().any(|s| source_matches_device(s, name));
    let parent = parent_disk(name);
    let is_system = system
        .iter()
        .any(|s| s == name || parent.as_ref() == Some(s) || Some(s.as_str()) == parent.as_deref());
    let is_app = app
        .iter()
        .any(|s| s == name || parent.as_ref() == Some(s) || Some(s.as_str()) == parent.as_deref());
    let in_use = !our_mounts.is_empty() || in_swap || !holders.is_empty();
    let media = format!("{:?}", kind).to_ascii_lowercase();
    let file_id = dev_file_id(&dev_path);

    Ok(TargetSnapshot {
        identity: DeviceIdentity {
            path: dev_path.clone(),
            canonical_path: dev_path,
            display_name: name.to_string(),
            model,
            serial,
            wwn,
            size,
            file_id,
            media,
        },
        kind,
        is_system,
        is_app_disk: is_app,
        mounts: our_mounts,
        in_use,
        is_raid_member: is_raid,
        is_lvm_member: is_lvm,
        backing_file: backing,
        is_symlink: false,
        is_regular_file: false,
        is_directory: false,
        is_block_device: true,
        logical_block_size: if block == 0 { 512 } else { block },
    })
}

fn class_block(name: &str) -> PathBuf {
    let p = Path::new("/sys/class/block").join(name);
    if p.exists() {
        p
    } else {
        // partitions live under /sys/block/<disk>/<part>
        if let Some(parent) = parent_disk(name) {
            Path::new("/sys/block").join(parent).join(name)
        } else {
            Path::new("/sys/block").join(name)
        }
    }
}

fn parent_disk(name: &str) -> Option<String> {
    let p = Path::new("/sys/class/block").join(name).join("partition");
    if !p.exists() && !Path::new("/sys/class/block").join(name).join("..").exists() {
        // try readlink /sys/class/block/name -> ../../block/sda/sda1
    }
    let link = Path::new("/sys/class/block").join(name);
    if let Ok(real) = fs::canonicalize(&link) {
        // .../block/sda/sda1 or .../block/sda
        if let Some(parent) = real.parent() {
            if parent.file_name().map(|s| s.to_string_lossy()) == Some("block".into()) {
                return Some(name.to_string());
            }
            if let Some(disk) = parent.file_name() {
                return Some(disk.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn system_disk_names() -> Vec<String> {
    let mut names = Vec::new();
    for p in [Path::new("/"), Path::new("/boot"), Path::new("/boot/efi")] {
        if p.exists() {
            for n in disks_for_path_inner(p) {
                if !names.contains(&n) {
                    names.push(n);
                }
            }
        }
    }
    names
}

struct Mount {
    source: String,
    mountpoint: PathBuf,
}

fn read_mounts() -> Vec<Mount> {
    let mut out = Vec::new();
    let text =
        fs::read_to_string("/proc/self/mountinfo").or_else(|_| fs::read_to_string("/proc/mounts"));
    let Ok(text) = text else { return out };
    for line in text.lines() {
        // mountinfo: ... - fstype source super
        if let Some(idx) = line.find(" - ") {
            let (left, right) = line.split_at(idx);
            let left_parts: Vec<&str> = left.split_whitespace().collect();
            let right_parts: Vec<&str> = right[3..].split_whitespace().collect();
            if left_parts.len() >= 5 && right_parts.len() >= 2 {
                let mp = unescape_mount(left_parts[4]);
                let source = right_parts[1].to_string();
                out.push(Mount {
                    source,
                    mountpoint: PathBuf::from(mp),
                });
            }
        } else {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                out.push(Mount {
                    source: parts[0].to_string(),
                    mountpoint: PathBuf::from(parts[1]),
                });
            }
        }
    }
    out
}

fn unescape_mount(s: &str) -> String {
    s.replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

fn read_swaps() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(f) = fs::File::open("/proc/swaps") {
        let reader = io::BufReader::new(f);
        for (i, line) in reader.lines().flatten().enumerate() {
            if i == 0 {
                continue;
            }
            if let Some(src) = line.split_whitespace().next() {
                out.push(src.to_string());
            }
        }
    }
    out
}

fn device_name_from_source(source: &str) -> Option<String> {
    let p = Path::new(source);
    if source.starts_with("/dev/") {
        let name = p.file_name()?.to_string_lossy().into_owned();
        // mapper names: resolve via /sys/class/block or just return
        if name.starts_with("dm-") || Path::new("/sys/class/block").join(&name).exists() {
            return Some(name);
        }
        // /dev/mapper/vg-lv -> try readlink
        if let Ok(real) = fs::canonicalize(source) {
            if let Some(n) = real.file_name() {
                return Some(n.to_string_lossy().into_owned());
            }
        }
        return Some(name);
    }
    None
}

fn source_matches_device(source: &str, name: &str) -> bool {
    if let Some(n) = device_name_from_source(source) {
        if n == name {
            return true;
        }
        if parent_disk(&n).as_deref() == Some(name) {
            return true;
        }
    }
    source.ends_with(&format!("/{name}")) || source == format!("/dev/{name}")
}

fn is_md_member(name: &str) -> bool {
    Path::new("/sys/class/block").join(name).join("md").exists()
        || Path::new("/sys/block").join(name).join("md").exists()
        || fs::read_to_string("/proc/mdstat")
            .ok()
            .map(|t| {
                t.split_whitespace()
                    .any(|w| w == name || w.starts_with(&format!("{name}[")))
            })
            .unwrap_or(false)
}

fn nvme_model(name: &str) -> Option<String> {
    let ctrl = nvme_ctrl(name)?;
    read_trim(&Path::new("/sys/class/nvme").join(ctrl).join("model"))
}

fn nvme_serial(name: &str) -> Option<String> {
    let ctrl = nvme_ctrl(name)?;
    read_trim(&Path::new("/sys/class/nvme").join(ctrl).join("serial"))
}

fn nvme_ctrl(name: &str) -> Option<String> {
    // nvme0n1 or nvme0n1p2 -> nvme0
    if !name.starts_with("nvme") {
        return None;
    }
    let bytes = name.as_bytes();
    let mut i = 4;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 4 {
        return None;
    }
    Some(name[..i].to_string())
}

fn read_trim(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn read_u64(path: &Path) -> Option<u64> {
    read_trim(path)?.parse().ok()
}

fn list_names(path: &Path) -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(rd) = fs::read_dir(path) {
        for e in rd.flatten() {
            v.push(e.file_name().to_string_lossy().into_owned());
        }
    }
    v
}

fn dev_file_id(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    fs::symlink_metadata(path).ok().map(|m| (m.dev(), m.ino()))
}
