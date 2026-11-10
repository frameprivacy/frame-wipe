//! Windows enumeration of physical drives and volumes.

use crate::device::{DeviceInfo, EnumerateOptions};
use crate::error::Error;
use crate::identity::DeviceIdentity;
use crate::planner::{MediaKind, TargetSnapshot};
use std::ffi::OsStr;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::{
    GetTokenInformation, OpenProcessToken, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, GetFileInformationByHandle,
    GetVolumePathNameW, GetVolumePathNamesForVolumeNameW, GetWindowsDirectoryW,
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;
use windows_sys::Win32::System::IO::DeviceIoControl;

const GENERIC_READ: u32 = 0x8000_0000;
const IOCTL_DISK_GET_LENGTH_INFO: u32 = 0x0007_405C;
const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D_1400;
const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x0056_0000;
const IOCTL_STORAGE_GET_DEVICE_NUMBER: u32 = 0x002D_1080;
const IOCTL_DISK_GET_DRIVE_LAYOUT_EX: u32 = 0x0007_0050;
const STORAGE_DEVICE_PROPERTY: u32 = 0;
const PROPERTY_STANDARD_QUERY: u32 = 0;

pub fn is_elevated() -> bool {
    unsafe {
        let mut token: HANDLE = 0;
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elev: TOKEN_ELEVATION = zeroed();
        let mut ret = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elev as *mut _ as *mut _,
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret,
        );
        CloseHandle(token);
        ok != 0 && elev.TokenIsElevated != 0
    }
}

pub fn enumerate(_opts: EnumerateOptions) -> Result<Vec<DeviceInfo>, Error> {
    let mut out = Vec::new();
    let system = system_disk_numbers();
    let app = disks_for_path(&std::env::current_exe().unwrap_or_default());
    let mounted = volume_mounts_by_disk();
    for n in 0..64u32 {
        let path = format!(r"\\.\PhysicalDrive{n}");
        match inspect_physical(&path, n, &system, &app, &mounted) {
            Ok(snap) => out.push(DeviceInfo { snapshot: snap }),
            Err(_) => {
                // Stop after a run of missing drives once we have seen at least one,
                // but keep scanning a bit so gaps don't hide later disks.
                if n > 8
                    && out.iter().all(|d| {
                        physical_num(&d.snapshot.identity.display_name)
                            .map(|x| x < n.saturating_sub(8))
                            .unwrap_or(true)
                    })
                {
                    // continue through 64 anyway; open failure is cheap
                }
            }
        }
    }
    Ok(out)
}

pub fn inspect_device(path: &Path) -> Result<TargetSnapshot, Error> {
    let system = system_disk_numbers();
    let app = disks_for_path(&std::env::current_exe().unwrap_or_default());
    let mounted = volume_mounts_by_disk();
    let s = path.to_string_lossy();
    if let Some(n) = parse_physical(&s) {
        let p = format!(r"\\.\PhysicalDrive{n}");
        return inspect_physical(&p, n, &system, &app, &mounted);
    }
    inspect_volume(path, &system, &app, &mounted)
}

pub fn disks_for_path(path: &Path) -> Vec<String> {
    disk_numbers_for_path(path)
        .into_iter()
        .map(|n| format!("PhysicalDrive{n}"))
        .collect()
}

fn physical_num(name: &str) -> Option<u32> {
    parse_physical(name)
}

fn parse_physical(s: &str) -> Option<u32> {
    let l = s.trim().trim_end_matches('\\');
    let l = l.rsplit('\\').next().unwrap_or(l);
    let l = l.strip_prefix("\\\\.\\").unwrap_or(l);
    let l = l.strip_prefix(r"\\.").unwrap_or(l);
    if let Some(rest) = l
        .strip_prefix("PhysicalDrive")
        .or_else(|| l.strip_prefix("physicaldrive"))
    {
        return rest.parse().ok();
    }
    let lower = l.to_ascii_lowercase();
    if let Some(idx) = lower.rfind("physicaldrive") {
        return lower[idx + 13..].parse().ok();
    }
    None
}

fn inspect_physical(
    path: &str,
    number: u32,
    system: &[u32],
    app_names: &[String],
    mounts: &[(u32, String)],
) -> Result<TargetSnapshot, Error> {
    let handle = open_ro(path)?;
    let result = inspect_physical_handle(handle, path, number, system, app_names, mounts);
    unsafe { CloseHandle(handle) };
    result
}

fn inspect_physical_handle(
    handle: HANDLE,
    path: &str,
    number: u32,
    system: &[u32],
    app_names: &[String],
    mounts: &[(u32, String)],
) -> Result<TargetSnapshot, Error> {
    let size = device_size(handle).unwrap_or(0);
    let (model, serial, bus) = device_descriptor(handle);
    let raid = looks_like_raid(handle, model.as_deref());
    let our_mounts: Vec<String> = mounts
        .iter()
        .filter(|(n, _)| *n == number)
        .map(|(_, m)| m.clone())
        .collect();
    let display = format!("PhysicalDrive{number}");
    let is_app = app_names.iter().any(|s| s.eq_ignore_ascii_case(&display));
    let kind = kind_from_bus(bus.as_deref(), model.as_deref());
    let lba = 512;
    Ok(TargetSnapshot {
        identity: DeviceIdentity {
            path: PathBuf::from(path),
            canonical_path: PathBuf::from(path),
            display_name: display,
            model,
            serial,
            wwn: None,
            size,
            file_id: None,
            media: format!("{:?}", kind).to_ascii_lowercase(),
        },
        kind,
        is_system: system.contains(&number),
        is_app_disk: is_app,
        mounts: our_mounts.clone(),
        in_use: !our_mounts.is_empty(),
        is_raid_member: raid,
        is_lvm_member: false,
        backing_file: None,
        is_symlink: false,
        is_regular_file: false,
        is_directory: false,
        is_block_device: true,
        logical_block_size: lba,
    })
}

fn inspect_volume(
    path: &Path,
    system: &[u32],
    app_names: &[String],
    mounts: &[(u32, String)],
) -> Result<TargetSnapshot, Error> {
    let raw = volume_raw_path(path);
    let handle = open_ro(&raw)?;
    let nums = disk_numbers_from_handle(handle);
    let size = device_size(handle).unwrap_or(0);
    let (model, serial, bus) = device_descriptor(handle);
    unsafe { CloseHandle(handle) };
    let display = volume_display_name(path);
    let is_system = nums.iter().any(|n| system.contains(n));
    let is_app = nums.iter().any(|n| {
        app_names
            .iter()
            .any(|s| s.eq_ignore_ascii_case(&format!("PhysicalDrive{n}")))
    });
    let our_mounts = if path.exists() {
        vec![path.display().to_string()]
    } else {
        mounts
            .iter()
            .filter(|(n, _)| nums.contains(n))
            .map(|(_, m)| m.clone())
            .collect()
    };
    let kind = kind_from_bus(bus.as_deref(), model.as_deref());
    Ok(TargetSnapshot {
        identity: DeviceIdentity {
            path: PathBuf::from(&raw),
            canonical_path: PathBuf::from(&raw),
            display_name: display,
            model,
            serial,
            wwn: None,
            size,
            file_id: None,
            media: format!("{:?}", kind).to_ascii_lowercase(),
        },
        kind,
        is_system,
        is_app_disk: is_app,
        mounts: our_mounts.clone(),
        in_use: !our_mounts.is_empty(),
        is_raid_member: false,
        is_lvm_member: false,
        backing_file: None,
        is_symlink: false,
        is_regular_file: false,
        is_directory: false,
        is_block_device: true,
        logical_block_size: 4096,
    })
}

fn volume_raw_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.starts_with(r"\\.\") || s.starts_with(r"\\?\") {
        return s.into_owned();
    }
    // C: or C:\ -> \\.\C:
    let bytes: Vec<u16> = OsStr::new(path).encode_wide().chain(Some(0)).collect();
    let mut buf = [0u16; 512];
    let ok = unsafe { GetVolumePathNameW(bytes.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
    if ok != 0 {
        let vol = utf16_to_string(&buf);
        let letter = vol.trim_end_matches('\\');
        if letter.len() >= 2 && letter.as_bytes()[1] == b':' {
            return format!(r"\\.\{}", letter);
        }
    }
    if s.len() >= 2 && s.as_bytes().get(1) == Some(&b':') {
        return format!(r"\\.\{}:", s.chars().next().unwrap_or('C'));
    }
    s.into_owned()
}

fn volume_display_name(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.len() >= 2 && s.as_bytes().get(1) == Some(&b':') {
        return format!("{}:", s.chars().next().unwrap_or('C').to_ascii_uppercase());
    }
    path.display().to_string()
}

fn kind_from_bus(bus: Option<&str>, model: Option<&str>) -> MediaKind {
    let blob = format!("{} {}", bus.unwrap_or(""), model.unwrap_or("")).to_ascii_lowercase();
    if blob.contains("nvme") || blob.contains("nvm express") {
        MediaKind::Nvme
    } else if blob.contains("ssd") || blob.contains("solid") {
        MediaKind::Ssd
    } else if blob.contains("virtual") || blob.contains("vhd") || blob.contains("vhdx") {
        MediaKind::Virtual
    } else if blob.contains("cdrom") || blob.contains("dvd") {
        MediaKind::Optical
    } else {
        MediaKind::Unknown
    }
}

fn open_ro(path: &str) -> Result<HANDLE, Error> {
    let w: Vec<u16> = OsStr::new(path).encode_wide().chain(Some(0)).collect();
    let h = unsafe {
        CreateFileW(
            w.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            0,
        )
    };
    if h == INVALID_HANDLE_VALUE {
        let err = std::io::Error::from_raw_os_error(unsafe { GetLastError() as i32 });
        return Err(Error::Open {
            path: PathBuf::from(path),
            detail: err.to_string(),
        });
    }
    Ok(h)
}

fn device_size(handle: HANDLE) -> Option<u64> {
    let mut len: i64 = 0;
    let mut ret = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_DISK_GET_LENGTH_INFO,
            ptr::null(),
            0,
            &mut len as *mut _ as *mut _,
            size_of::<i64>() as u32,
            &mut ret,
            ptr::null_mut(),
        )
    };
    if ok != 0 && len > 0 {
        Some(len as u64)
    } else {
        None
    }
}

#[repr(C)]
struct StoragePropertyQuery {
    property_id: u32,
    query_type: u32,
    additional: [u8; 1],
}

fn device_descriptor(handle: HANDLE) -> (Option<String>, Option<String>, Option<String>) {
    let mut query = StoragePropertyQuery {
        property_id: STORAGE_DEVICE_PROPERTY,
        query_type: PROPERTY_STANDARD_QUERY,
        additional: [0],
    };
    let mut buf = [0u8; 1024];
    let mut ret = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            &mut query as *mut _ as *mut _,
            size_of::<StoragePropertyQuery>() as u32,
            buf.as_mut_ptr() as *mut _,
            buf.len() as u32,
            &mut ret,
            ptr::null_mut(),
        )
    };
    if ok == 0 || ret < 28 {
        return (None, None, None);
    }
    // STORAGE_DEVICE_DESCRIPTOR offsets: ProductIdOffset at 16, SerialNumberOffset at 24, BusType at 28.
    let product_off = u32_at(&buf, 16);
    let serial_off = u32_at(&buf, 24);
    let bus_ty = if ret as usize >= 29 { buf[28] } else { 0 };
    let model = offset_str(&buf, product_off, ret);
    let serial = offset_str(&buf, serial_off, ret);
    let bus = bus_name(bus_ty);
    (model, serial, bus)
}

fn u32_at(buf: &[u8], off: usize) -> u32 {
    if off + 4 <= buf.len() {
        u32::from_le_bytes(buf[off..off + 4].try_into().unwrap_or([0; 4]))
    } else {
        0
    }
}

fn offset_str(buf: &[u8], off: u32, len: u32) -> Option<String> {
    if off == 0 || off as usize >= len as usize {
        return None;
    }
    let slice = &buf[off as usize..len as usize];
    let end = slice.iter().position(|b| *b == 0).unwrap_or(slice.len());
    let s = String::from_utf8_lossy(&slice[..end]).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn bus_name(ty: u8) -> Option<String> {
    Some(
        match ty {
            1 => "SCSI",
            4 => "ATA",
            7 => "USB",
            8 => "RAID",
            11 => "Fibre",
            17 => "NVMe",
            _ => return None,
        }
        .to_string(),
    )
}

fn looks_like_raid(handle: HANDLE, model: Option<&str>) -> bool {
    if let Some(m) = model {
        let l = m.to_ascii_lowercase();
        if l.contains("raid")
            || l.contains("storage spaces")
            || l.contains("virtual") && l.contains("microsoft")
        {
            return true;
        }
    }
    // PARTITION_STYLE / LDM: best-effort via drive layout buffer (first bytes).
    let mut buf = [0u8; 4096];
    let mut ret = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_DISK_GET_DRIVE_LAYOUT_EX,
            ptr::null(),
            0,
            buf.as_mut_ptr() as *mut _,
            buf.len() as u32,
            &mut ret,
            ptr::null_mut(),
        )
    };
    if ok != 0 && ret >= 4 {
        // PartitionStyle: 0=MBR, 1=GPT, 2=RAW (LDM often 0 with LDM type entries)
        let style = u32_at(&buf, 0);
        if style > 2 {
            return true;
        }
    }
    let _ = handle;
    false
}

fn system_disk_numbers() -> Vec<u32> {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) };
    if n == 0 {
        return Vec::new();
    }
    let win = utf16_to_string(&buf);
    disk_numbers_for_path(Path::new(&win))
}

fn disk_numbers_for_path(path: &Path) -> Vec<u32> {
    if path.as_os_str().is_empty() {
        return Vec::new();
    }
    let raw = volume_raw_path(path);
    let Ok(handle) = open_ro(&raw) else {
        return Vec::new();
    };
    let nums = disk_numbers_from_handle(handle);
    unsafe { CloseHandle(handle) };
    nums
}

#[repr(C)]
struct DiskExtent {
    disk_number: u32,
    starting_offset: i64,
    extent_length: i64,
}

#[repr(C)]
struct VolumeDiskExtentsHeader {
    number: u32,
    extent: DiskExtent,
}

fn disk_numbers_from_handle(handle: HANDLE) -> Vec<u32> {
    let mut buf = [0u8; 1024];
    let mut ret = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
            ptr::null(),
            0,
            buf.as_mut_ptr() as *mut _,
            buf.len() as u32,
            &mut ret,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        // Try STORAGE_DEVICE_NUMBER
        let mut sdn = [0u8; 12];
        let mut ret2 = 0u32;
        let ok2 = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_STORAGE_GET_DEVICE_NUMBER,
                ptr::null(),
                0,
                sdn.as_mut_ptr() as *mut _,
                sdn.len() as u32,
                &mut ret2,
                ptr::null_mut(),
            )
        };
        if ok2 != 0 && ret2 >= 8 {
            return vec![u32_at(&sdn, 4)];
        }
        return Vec::new();
    }
    if ret < 4 {
        return Vec::new();
    }
    let count = u32_at(&buf, 0) as usize;
    let mut nums = Vec::new();
    let mut off = 8; // alignment after NumberOfDiskExtents, then first extent at 8
                     // Actually VOLUME_DISK_EXTENTS: DWORD NumberOfDiskExtents; DISK_EXTENT Extents[1];
                     // DISK_EXTENT is 24 bytes (u32 + pad + i64 + i64), first extent starts at offset 8.
    for _ in 0..count {
        if off + 4 > buf.len() {
            break;
        }
        nums.push(u32_at(&buf, off));
        off += 24;
    }
    let _ = size_of::<VolumeDiskExtentsHeader>();
    nums
}

fn volume_mounts_by_disk() -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let mut name = [0u16; 128];
    let handle = unsafe { FindFirstVolumeW(name.as_mut_ptr(), name.len() as u32) };
    if handle == INVALID_HANDLE_VALUE {
        return out;
    }
    loop {
        let vol = utf16_to_string(&name);
        let mut paths = [0u16; 1024];
        let mut needed = 0u32;
        let ok = unsafe {
            GetVolumePathNamesForVolumeNameW(
                name.as_ptr(),
                paths.as_mut_ptr(),
                paths.len() as u32,
                &mut needed,
            )
        };
        if ok != 0 {
            let p = utf16_to_string(&paths);
            if !p.is_empty() {
                let raw = volume_raw_path(Path::new(&vol));
                if let Ok(h) = open_ro(&raw) {
                    for n in disk_numbers_from_handle(h) {
                        out.push((n, p.trim_end_matches('\0').to_string()));
                    }
                    unsafe { CloseHandle(h) };
                }
            }
        }
        if unsafe { FindNextVolumeW(handle, name.as_mut_ptr(), name.len() as u32) } == 0 {
            break;
        }
    }
    unsafe { FindVolumeClose(handle) };
    out
}

fn utf16_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

// Silence unused import if Metadata path is unused.
#[allow(dead_code)]
fn _file_info(handle: HANDLE) -> Option<BY_HANDLE_FILE_INFORMATION> {
    let mut info = unsafe { zeroed() };
    let ok = unsafe { GetFileInformationByHandle(handle, &mut info) };
    if ok != 0 {
        Some(info)
    } else {
        None
    }
}
