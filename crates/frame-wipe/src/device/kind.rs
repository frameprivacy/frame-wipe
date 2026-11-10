//! Classify media from platform signals. Kept free of live /sys or disk access
//! so tests can cover USB flash drives that report as non-rotational.

use crate::planner::MediaKind;
use std::path::Path;

/// Linux sysfs: USB flash typically has `queue/rotational` = 0, which would
/// look like an SSD if the USB bus is not checked first.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn from_linux_signals(
    name: &str,
    rotational: Option<&str>,
    removable: Option<&str>,
    on_usb: bool,
    has_backing: bool,
) -> MediaKind {
    if name.starts_with("loop") {
        return MediaKind::Loop;
    }
    if name.starts_with("ram") || name.starts_with("zram") {
        return MediaKind::Ram;
    }
    if has_backing {
        return MediaKind::Virtual;
    }
    if name.starts_with("nvme") {
        return MediaKind::Nvme;
    }
    if name.starts_with("sr") {
        return MediaKind::Optical;
    }
    if on_usb || removable == Some("1") {
        return MediaKind::Usb;
    }
    if rotational == Some("0") {
        return MediaKind::Ssd;
    }
    if rotational == Some("1") {
        return MediaKind::Hdd;
    }
    MediaKind::Unknown
}

/// True when a sysfs device path sits under a USB controller
/// (`.../usb1/...`, `.../usb/...`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn sysfs_path_is_usb(path: &str) -> bool {
    Path::new(path).components().any(|c| {
        let c = c.as_os_str().to_string_lossy().to_ascii_lowercase();
        if c == "usb" {
            return true;
        }
        let Some(rest) = c.strip_prefix("usb") else {
            return false;
        };
        !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit())
    })
}

/// Windows STORAGE_BUS_TYPE plus the model string.
/// A USB flash drive's model does not usually contain "SSD", but a USB SSD
/// enclosure might; the bus still wins so it is not shown as an internal SSD.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn from_windows_bus(bus: Option<&str>, model: Option<&str>) -> MediaKind {
    let bus_l = bus.unwrap_or("").to_ascii_lowercase();
    let model_l = model.unwrap_or("").to_ascii_lowercase();
    let blob = format!("{bus_l} {model_l}");
    if blob.contains("nvme") || blob.contains("nvm express") {
        MediaKind::Nvme
    } else if bus_l == "usb" || bus_l.contains("usb") {
        MediaKind::Usb
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

/// macOS `diskutil` protocol and Solid State flag. USB flash is often
/// reported as solid-state, so the USB protocol is checked first.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn from_macos_signals(
    protocol: &str,
    solid_state: bool,
    virtual_media: bool,
    id: &str,
    have_info: bool,
) -> MediaKind {
    let proto = protocol.to_ascii_lowercase();
    if virtual_media {
        MediaKind::Virtual
    } else if proto == "pci-express" || id.contains("nvme") || proto.contains("nvme") {
        MediaKind::Nvme
    } else if proto == "usb" || proto.contains("usb") {
        MediaKind::Usb
    } else if solid_state {
        MediaKind::Ssd
    } else if proto.contains("disc") {
        MediaKind::Optical
    } else if have_info {
        MediaKind::Hdd
    } else {
        MediaKind::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_usb_flash_with_rotational_zero_is_usb_not_ssd() {
        // Typical SanDisk USB stick: sd-name, rotational=0, removable, under usbN.
        let kind = from_linux_signals("sdb", Some("0"), Some("1"), true, false);
        assert_eq!(kind, MediaKind::Usb);
        assert_eq!(kind.label(), "USB");
        assert_ne!(kind, MediaKind::Ssd);
    }

    #[test]
    fn linux_internal_nonrotational_disk_is_still_ssd() {
        assert_eq!(
            from_linux_signals("sda", Some("0"), Some("0"), false, false),
            MediaKind::Ssd
        );
    }

    #[test]
    fn linux_rotational_disk_is_hdd() {
        assert_eq!(
            from_linux_signals("sda", Some("1"), Some("0"), false, false),
            MediaKind::Hdd
        );
    }

    #[test]
    fn linux_nvme_stays_nvme_even_if_nonrotational() {
        assert_eq!(
            from_linux_signals("nvme0n1", Some("0"), Some("0"), false, false),
            MediaKind::Nvme
        );
    }

    #[test]
    fn linux_usb_path_is_detected_in_sysfs() {
        assert!(sysfs_path_is_usb(
            "/sys/devices/pci0000:00/0000:00:14.0/usb1/1-3/1-3:1.0/host4/target4:0:0/4:0:0:0/block/sdb"
        ));
        assert!(sysfs_path_is_usb("/sys/devices/platform/soc/usb/block/sda"));
        assert!(!sysfs_path_is_usb(
            "/sys/devices/pci0000:00/0000:00:17.0/ata1/host0/target0:0:0/0:0:0:0/block/sda"
        ));
        assert!(!sysfs_path_is_usb(
            "/sys/devices/pci0000:00/0000:00:1d.0/usbmix/block/sda"
        ));
    }

    #[test]
    fn linux_removable_without_usb_path_is_still_removable() {
        // SD readers and similar: removable=1, rotational=0, no USB path.
        assert_eq!(
            from_linux_signals("mmcblk0", Some("0"), Some("1"), false, false),
            MediaKind::Usb
        );
    }

    #[test]
    fn windows_sandisk_usb_is_usb_not_ssd_or_unknown() {
        let kind = from_windows_bus(Some("USB"), Some("SanDisk USB 3.2"));
        assert_eq!(kind, MediaKind::Usb);
        assert_ne!(kind, MediaKind::Ssd);
    }

    #[test]
    fn windows_usb_enclosure_is_usb_even_if_model_says_ssd() {
        assert_eq!(
            from_windows_bus(Some("USB"), Some("Samsung SSD T7")),
            MediaKind::Usb
        );
    }

    #[test]
    fn windows_internal_ssd_stays_ssd() {
        assert_eq!(
            from_windows_bus(Some("ATA"), Some("Samsung SSD 990 PRO")),
            MediaKind::Ssd
        );
    }

    #[test]
    fn macos_usb_flash_reported_solid_state_is_usb() {
        let kind = from_macos_signals("USB", true, false, "disk2", true);
        assert_eq!(kind, MediaKind::Usb);
        assert_ne!(kind, MediaKind::Ssd);
    }

    #[test]
    fn macos_internal_solid_state_stays_ssd_or_nvme() {
        assert_eq!(
            from_macos_signals("PCI-Express", true, false, "disk0", true),
            MediaKind::Nvme
        );
        assert_eq!(
            from_macos_signals("SATA", true, false, "disk0", true),
            MediaKind::Ssd
        );
    }
}
