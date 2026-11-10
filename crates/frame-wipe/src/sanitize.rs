//! Firmware sanitize / secure erase. Only offered when the device reports support.
//! This is a Purge-class operation in the sense of NIST SP 800-88; the tool itself is not certified.

use crate::error::Error;
use crate::planner::{plan, MediaKind, Scope};
use crate::report::{Outcome, Report};
use crate::safety::SafetyFlags;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use time::OffsetDateTime;

pub fn capabilities(path: &Path) -> Result<SanitizeCaps, Error> {
    let snap = crate::device::inspect(path)?;
    Ok(detect(&snap))
}

#[derive(Debug, Clone)]
pub struct SanitizeCaps {
    pub supported: bool,
    pub method: Option<String>,
    pub detail: String,
}

fn detect(snap: &crate::planner::TargetSnapshot) -> SanitizeCaps {
    if snap.is_regular_file || snap.kind == MediaKind::Image {
        return SanitizeCaps {
            supported: false,
            method: None,
            detail: "firmware sanitize is not available on disk image files".into(),
        };
    }
    #[cfg(target_os = "linux")]
    {
        if snap.kind == MediaKind::Nvme || snap.identity.display_name.starts_with("nvme") {
            if let Some(caps) = linux_nvme_caps(&snap.identity.display_name) {
                return caps;
            }
            return SanitizeCaps {
                supported: false,
                method: Some("nvme-sanitize".into()),
                detail: "this looks like an NVMe device, but the controller sanitize capabilities could not be read. On Linux, firmware sanitize may be issued later with nvme-cli once support is confirmed.".into(),
            };
        }
        if snap.kind == MediaKind::Ssd {
            return SanitizeCaps {
                supported: false,
                method: Some("ata-secure-erase".into()),
                detail: "ATA Secure Erase is not issued by this tool. A frozen security lock is common, and a wrong command can make a drive unusable. Use the vendor tool if you need a firmware erase.".into(),
            };
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = snap;
    }
    SanitizeCaps {
        supported: false,
        method: None,
        detail: "firmware sanitize is not implemented on this platform in this version. Overwrite mode can still be used, and is closer to NIST SP 800-88 Clear than Purge, especially on SSDs.".into(),
    }
}

#[cfg(target_os = "linux")]
fn linux_nvme_caps(name: &str) -> Option<SanitizeCaps> {
    // nvme0n1 / nvme0n1p1 -> /sys/class/nvme/nvme0
    let ctrl = {
        let mut i = 4;
        let b = name.as_bytes();
        if !name.starts_with("nvme") {
            return None;
        }
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == 4 {
            return None;
        }
        &name[..i]
    };
    let base = std::path::Path::new("/sys/class/nvme").join(ctrl);
    // sanicap is not always exported; subnqn / model at least tell us it is NVMe.
    let model = std::fs::read_to_string(base.join("model"))
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    Some(SanitizeCaps {
        supported: false,
        method: Some("nvme-sanitize".into()),
        detail: format!(
            "NVMe controller {ctrl} ({model}) detected. This version reports capability only and does not send a Sanitize or Format command. Use nvme-cli (`nvme sanitize`) if you need a firmware purge, after checking the drive manual."
        ),
    })
}

/// Plan and refuse or (in later versions) issue a firmware sanitize. This version never
/// sends a sanitize command; it still runs every safety check so the CLI path is real.
pub fn sanitize(
    path: &Path,
    flags: SafetyFlags,
    cancel: &Arc<AtomicBool>,
) -> Result<Report, Error> {
    let _ = cancel;
    let snap = crate::device::inspect(path)?;
    let planned = plan(
        &snap,
        Scope::FirmwareSanitize,
        crate::method::Method::zeros(),
        None,
        &flags,
    )?;
    let caps = detect(&snap);
    if planned.dry_run {
        return Ok(crate::report::from_plan(
            &planned,
            Outcome::DryRun,
            OffsetDateTime::now_utc(),
            OffsetDateTime::now_utc(),
            0,
            Vec::new(),
            Some(0),
        ));
    }
    Err(Error::FirmwareNotSupported(caps.detail))
}
