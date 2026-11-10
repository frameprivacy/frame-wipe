//! Firmware sanitize and secure erase.
//!
//! Linux can issue NVMe Sanitize / Format NVM and ATA SECURITY ERASE via ioctl,
//! only after the planner accepts the request and only when
//! `FRAME_WIPE_FORBID_REAL_DISKS` is not set. macOS and Windows report
//! capability only in this version. Tests use a mock backend and never talk to
//! a device.

use crate::error::Error;
use crate::planner::{plan, MediaKind, Plan, Scope, TargetSnapshot};
use crate::report::{Outcome, Report};
use crate::safety::SafetyFlags;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use time::OffsetDateTime;

/// True when firmware commands must not be sent (CI, tests, and any run that
/// sets `FRAME_WIPE_FORBID_REAL_DISKS=1`).
pub fn firmware_commands_forbidden() -> bool {
    matches!(
        std::env::var("FRAME_WIPE_FORBID_REAL_DISKS"),
        Ok(v) if v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SanitizeAction {
    NvmeSanitizeBlock,
    NvmeSanitizeCrypto,
    NvmeFormat,
    AtaSecureErase,
}

impl SanitizeAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::NvmeSanitizeBlock => "nvme-sanitize-block",
            Self::NvmeSanitizeCrypto => "nvme-sanitize-crypto",
            Self::NvmeFormat => "nvme-format",
            Self::AtaSecureErase => "ata-secure-erase",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SanitizeCaps {
    pub supported: bool,
    pub method: Option<String>,
    pub action: Option<SanitizeAction>,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct SanitizeOutcome {
    pub method: String,
    pub detail: String,
}

/// Issuing backend. Production uses [`PlatformBackend`]. Tests inject a mock.
pub trait Backend {
    fn caps(&self, snap: &TargetSnapshot) -> SanitizeCaps;
    fn issue(
        &self,
        snap: &TargetSnapshot,
        action: SanitizeAction,
    ) -> Result<SanitizeOutcome, Error>;
}

/// The host platform's firmware path. Linux may issue ioctls; others never do.
pub struct PlatformBackend;

impl Backend for PlatformBackend {
    fn caps(&self, snap: &TargetSnapshot) -> SanitizeCaps {
        detect(snap)
    }

    fn issue(
        &self,
        snap: &TargetSnapshot,
        action: SanitizeAction,
    ) -> Result<SanitizeOutcome, Error> {
        if firmware_commands_forbidden() {
            return Err(Error::FirmwareNotSupported(
                "firmware commands are disabled while FRAME_WIPE_FORBID_REAL_DISKS is set, and are never issued in tests".into(),
            ));
        }
        #[cfg(target_os = "linux")]
        {
            return linux::issue(snap, action);
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (snap, action);
            Err(Error::FirmwareNotSupported(
                "firmware sanitize is not implemented on this platform in this version".into(),
            ))
        }
    }
}

pub fn capabilities(path: &Path) -> Result<SanitizeCaps, Error> {
    let snap = crate::device::inspect(path)?;
    Ok(detect(&snap))
}

fn detect(snap: &TargetSnapshot) -> SanitizeCaps {
    if snap.is_regular_file || snap.kind == MediaKind::Image {
        return SanitizeCaps {
            supported: false,
            method: None,
            action: None,
            detail: "firmware sanitize is not available on disk image files".into(),
        };
    }
    #[cfg(target_os = "linux")]
    {
        return linux::detect(snap);
    }
    #[cfg(not(target_os = "linux"))]
    {
        SanitizeCaps {
            supported: false,
            method: None,
            action: None,
            detail: "firmware sanitize is not implemented on this platform in this version. Overwrite mode can still be used.".into(),
        }
    }
}

/// Plan and, when supported and allowed, issue a firmware sanitize.
pub fn sanitize(
    path: &Path,
    flags: SafetyFlags,
    cancel: &Arc<AtomicBool>,
) -> Result<Report, Error> {
    sanitize_with(&PlatformBackend, path, flags, cancel)
}

pub fn sanitize_with<B: Backend>(
    backend: &B,
    path: &Path,
    flags: SafetyFlags,
    cancel: &Arc<AtomicBool>,
) -> Result<Report, Error> {
    let snap = crate::device::inspect(path)?;
    let planned = plan(
        &snap,
        Scope::FirmwareSanitize,
        crate::method::Method::zeros(),
        None,
        &flags,
    )?;
    finish_sanitize(backend, &snap, planned, cancel)
}

/// Run a firmware sanitize against an already-inspected snapshot. Used by tests
/// with a mock backend so no device is opened for issue().
pub fn finish_sanitize<B: Backend>(
    backend: &B,
    snap: &TargetSnapshot,
    planned: Plan,
    cancel: &Arc<AtomicBool>,
) -> Result<Report, Error> {
    let started = OffsetDateTime::now_utc();
    let caps = backend.caps(snap);
    if planned.dry_run {
        return Ok(crate::report::from_plan(
            &planned,
            Outcome::DryRun,
            started,
            OffsetDateTime::now_utc(),
            0,
            Vec::new(),
            Some(0),
        ));
    }
    if cancel.load(Ordering::SeqCst) {
        return Err(Error::Cancelled);
    }
    if !caps.supported {
        return Err(Error::FirmwareNotSupported(caps.detail));
    }
    let action = caps.action.ok_or_else(|| {
        Error::FirmwareNotSupported(
            "the device reported sanitize support but no concrete action was selected".into(),
        )
    })?;
    let issued = backend.issue(snap, action)?;
    let finished = OffsetDateTime::now_utc();
    let mut report = crate::report::from_plan(
        &planned,
        Outcome::Success,
        started,
        finished,
        planned.size,
        Vec::new(),
        Some(u64::try_from((finished - started).whole_milliseconds().max(0)).unwrap_or(0)),
    );
    report.notes.push(format!(
        "firmware {} completed: {}",
        issued.method, issued.detail
    ));
    Ok(report)
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{SanitizeAction, SanitizeCaps, SanitizeOutcome};
    use crate::error::Error;
    use crate::planner::{MediaKind, TargetSnapshot};
    use std::fs::OpenOptions;
    use std::io::{self, ErrorKind};
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    use std::path::{Path, PathBuf};

    const NVME_IOCTL_ADMIN_CMD: libc::c_ulong = 0xC048_4E41;
    const NVME_ADMIN_IDENTIFY: u8 = 0x06;
    const NVME_ADMIN_FORMAT: u8 = 0x80;
    const NVME_ADMIN_SANITIZE: u8 = 0x84;
    const CNS_CONTROLLER: u32 = 1;
    const SANACT_BLOCK: u32 = 0x2;
    const SANACT_CRYPTO: u32 = 0x4;

    // ATA / SCSI
    const SG_IO: libc::c_ulong = 0x2285;
    const SG_DXFER_FROM_DEV: i32 = -3;
    const SG_DXFER_NONE: i32 = -1;
    const SG_DXFER_TO_DEV: i32 = -2;

    #[repr(C)]
    struct NvmeAdminCmd {
        opcode: u8,
        flags: u8,
        rsvd1: u16,
        nsid: u32,
        cdw2: u32,
        cdw3: u32,
        metadata: u64,
        addr: u64,
        metadata_len: u32,
        data_len: u32,
        cdw10: u32,
        cdw11: u32,
        cdw12: u32,
        cdw13: u32,
        cdw14: u32,
        cdw15: u32,
        timeout_ms: u32,
        result: u32,
    }

    #[repr(C)]
    struct SgIoHdr {
        interface_id: i32,
        dxfer_direction: i32,
        cmd_len: u8,
        mx_sb_len: u8,
        iovec_count: u16,
        dxfer_len: u32,
        dxferp: *mut u8,
        cmdp: *mut u8,
        sbp: *mut u8,
        timeout: u32,
        flags: u32,
        pack_id: i32,
        usr_ptr: *mut u8,
        status: u8,
        masked_status: u8,
        msg_status: u8,
        sb_len_wr: u8,
        host_status: u16,
        driver_status: u16,
        resid: i32,
        duration: u32,
        info: u32,
    }

    pub fn detect(snap: &TargetSnapshot) -> SanitizeCaps {
        if snap.kind == MediaKind::Nvme || snap.identity.display_name.starts_with("nvme") {
            return nvme_caps(snap);
        }
        if matches!(
            snap.kind,
            MediaKind::Ssd | MediaKind::Hdd | MediaKind::Unknown
        ) && snap.is_block_device
        {
            return ata_caps(snap);
        }
        SanitizeCaps {
            supported: false,
            method: None,
            action: None,
            detail: "firmware sanitize is not available for this kind of device".into(),
        }
    }

    pub fn issue(snap: &TargetSnapshot, action: SanitizeAction) -> Result<SanitizeOutcome, Error> {
        if super::firmware_commands_forbidden() {
            return Err(Error::FirmwareNotSupported(
                "firmware commands are disabled while FRAME_WIPE_FORBID_REAL_DISKS is set".into(),
            ));
        }
        match action {
            SanitizeAction::NvmeSanitizeBlock
            | SanitizeAction::NvmeSanitizeCrypto
            | SanitizeAction::NvmeFormat => issue_nvme(snap, action),
            SanitizeAction::AtaSecureErase => issue_ata(snap),
        }
    }

    fn nvme_ctrl_name(name: &str) -> Option<&str> {
        if !name.starts_with("nvme") {
            return None;
        }
        let b = name.as_bytes();
        let mut i = 4;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == 4 {
            None
        } else {
            Some(&name[..i])
        }
    }

    fn nvme_char_path(snap: &TargetSnapshot) -> PathBuf {
        if let Some(ctrl) = nvme_ctrl_name(&snap.identity.display_name) {
            return PathBuf::from(format!("/dev/{ctrl}"));
        }
        snap.identity.path.clone()
    }

    fn nvme_caps(snap: &TargetSnapshot) -> SanitizeCaps {
        match identify_controller(&nvme_char_path(snap)) {
            Ok(id) => parse_nvme_identify(snap, &id),
            Err(e) => {
                let model = snap
                    .identity
                    .model
                    .clone()
                    .unwrap_or_else(|| "unknown".into());
                SanitizeCaps {
                    supported: false,
                    method: Some("nvme-sanitize".into()),
                    action: None,
                    detail: format!(
                        "NVMe device {} ({model}) was found, but controller identify failed ({e}). No firmware command will be sent.",
                        snap.identity.display_name
                    ),
                }
            }
        }
    }

    fn parse_nvme_identify(snap: &TargetSnapshot, id: &[u8]) -> SanitizeCaps {
        if id.len() < 332 {
            return SanitizeCaps {
                supported: false,
                method: Some("nvme-sanitize".into()),
                action: None,
                detail: "NVMe Identify Controller returned a short buffer".into(),
            };
        }
        // SANICAP is a 32-bit little-endian field at byte 328.
        let sanicap = u32::from_le_bytes([id[328], id[329], id[330], id[331]]);
        let crypto = (sanicap & 0x1) != 0;
        let block = (sanicap & 0x2) != 0;
        let overwrite = (sanicap & 0x4) != 0;
        // OACS bit 1 (byte 256, bit 1) is Format NVM support.
        let oacs = u16::from_le_bytes([id[256], id[257]]);
        let format = (oacs & 0x2) != 0;
        let model = snap.identity.model.as_deref().unwrap_or("unknown");
        if crypto {
            SanitizeCaps {
                supported: true,
                method: Some(SanitizeAction::NvmeSanitizeCrypto.label().into()),
                action: Some(SanitizeAction::NvmeSanitizeCrypto),
                detail: format!(
                    "NVMe Sanitize Crypto Erase is supported on {} ({model}). This permanently erases media encryption keys. A typed confirmation of the device name is required. This tool is not NIST certified.",
                    snap.identity.display_name
                ),
            }
        } else if block {
            SanitizeCaps {
                supported: true,
                method: Some(SanitizeAction::NvmeSanitizeBlock.label().into()),
                action: Some(SanitizeAction::NvmeSanitizeBlock),
                detail: format!(
                    "NVMe Sanitize Block Erase is supported on {} ({model}). A typed confirmation of the device name is required. This tool is not NIST certified.",
                    snap.identity.display_name
                ),
            }
        } else if format {
            SanitizeCaps {
                supported: true,
                method: Some(SanitizeAction::NvmeFormat.label().into()),
                action: Some(SanitizeAction::NvmeFormat),
                detail: format!(
                    "NVMe Format NVM is supported on {} ({model}) (Sanitize is not). A typed confirmation of the device name is required. This tool is not NIST certified.",
                    snap.identity.display_name
                ),
            }
        } else {
            let _ = overwrite;
            SanitizeCaps {
                supported: false,
                method: Some("nvme-sanitize".into()),
                action: None,
                detail: format!(
                    "NVMe controller {} ({model}) does not report Sanitize or Format NVM support (SANICAP=0x{sanicap:08x}).",
                    snap.identity.display_name
                ),
            }
        }
    }

    fn identify_controller(path: &Path) -> Result<[u8; 4096], String> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .map_err(|e| format!("open {}: {e}", path.display()))?;
        let mut buf = [0u8; 4096];
        let mut cmd = NvmeAdminCmd {
            opcode: NVME_ADMIN_IDENTIFY,
            flags: 0,
            rsvd1: 0,
            nsid: 0,
            cdw2: 0,
            cdw3: 0,
            metadata: 0,
            addr: buf.as_mut_ptr() as u64,
            metadata_len: 0,
            data_len: 4096,
            cdw10: CNS_CONTROLLER,
            cdw11: 0,
            cdw12: 0,
            cdw13: 0,
            cdw14: 0,
            cdw15: 0,
            timeout_ms: 10_000,
            result: 0,
        };
        let rc = unsafe { libc::ioctl(file.as_raw_fd(), NVME_IOCTL_ADMIN_CMD, &mut cmd as *mut _) };
        if rc < 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        Ok(buf)
    }

    fn issue_nvme(snap: &TargetSnapshot, action: SanitizeAction) -> Result<SanitizeOutcome, Error> {
        let path = nvme_char_path(snap);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| Error::Open {
                path: path.clone(),
                detail: format!("open for firmware command failed: {e}"),
            })?;
        let mut cmd = match action {
            SanitizeAction::NvmeSanitizeCrypto => NvmeAdminCmd {
                opcode: NVME_ADMIN_SANITIZE,
                flags: 0,
                rsvd1: 0,
                nsid: 0xFFFF_FFFF,
                cdw2: 0,
                cdw3: 0,
                metadata: 0,
                addr: 0,
                metadata_len: 0,
                data_len: 0,
                cdw10: SANACT_CRYPTO,
                cdw11: 0,
                cdw12: 0,
                cdw13: 0,
                cdw14: 0,
                cdw15: 0,
                timeout_ms: 0,
                result: 0,
            },
            SanitizeAction::NvmeSanitizeBlock => NvmeAdminCmd {
                opcode: NVME_ADMIN_SANITIZE,
                flags: 0,
                rsvd1: 0,
                nsid: 0xFFFF_FFFF,
                cdw2: 0,
                cdw3: 0,
                metadata: 0,
                addr: 0,
                metadata_len: 0,
                data_len: 0,
                cdw10: SANACT_BLOCK,
                cdw11: 0,
                cdw12: 0,
                cdw13: 0,
                cdw14: 0,
                cdw15: 0,
                timeout_ms: 0,
                result: 0,
            },
            SanitizeAction::NvmeFormat => NvmeAdminCmd {
                // SES = 1 (user data erase) in bits 11:9 of CDW10; nsid is this namespace.
                opcode: NVME_ADMIN_FORMAT,
                flags: 0,
                rsvd1: 0,
                nsid: 1,
                cdw2: 0,
                cdw3: 0,
                metadata: 0,
                addr: 0,
                metadata_len: 0,
                data_len: 0,
                cdw10: 1 << 9,
                cdw11: 0,
                cdw12: 0,
                cdw13: 0,
                cdw14: 0,
                cdw15: 0,
                timeout_ms: 0,
                result: 0,
            },
            SanitizeAction::AtaSecureErase => {
                return Err(Error::FirmwareNotSupported(
                    "internal error: ATA action routed to NVMe issuer".into(),
                ));
            }
        };
        let rc = unsafe { libc::ioctl(file.as_raw_fd(), NVME_IOCTL_ADMIN_CMD, &mut cmd as *mut _) };
        if rc < 0 {
            let err = io::Error::last_os_error();
            return Err(Error::Message(format!(
                "NVMe {} ioctl failed on {}: {err}",
                action.label(),
                path.display()
            )));
        }
        Ok(SanitizeOutcome {
            method: action.label().into(),
            detail: format!(
                "ioctl on {} returned success (status 0x{:08x})",
                path.display(),
                cmd.result
            ),
        })
    }

    fn ata_caps(snap: &TargetSnapshot) -> SanitizeCaps {
        match ata_identify(&snap.identity.path) {
            Ok(id) => parse_ata_identify(snap, &id),
            Err(e) => SanitizeCaps {
                supported: false,
                method: Some(SanitizeAction::AtaSecureErase.label().into()),
                action: None,
                detail: format!(
                    "ATA Identify failed on {} ({e}). ATA Secure Erase will not be issued.",
                    snap.identity.display_name
                ),
            },
        }
    }

    fn parse_ata_identify(snap: &TargetSnapshot, words: &[u16; 256]) -> SanitizeCaps {
        // Word 82 bit 1: SECURITY feature set supported.
        let security_supported = (words[82] & (1 << 1)) != 0;
        // Word 128: security status. bit 3 frozen, bit 1 enabled, bit 0 supported.
        let status = words[128];
        let frozen = (status & (1 << 3)) != 0;
        let enabled = (status & (1 << 1)) != 0;
        let status_supported = (status & (1 << 0)) != 0;
        let model = snap.identity.model.as_deref().unwrap_or("unknown");
        if !security_supported && !status_supported {
            return SanitizeCaps {
                supported: false,
                method: Some(SanitizeAction::AtaSecureErase.label().into()),
                action: None,
                detail: format!(
                    "{} ({model}) does not report the ATA SECURITY feature set.",
                    snap.identity.display_name
                ),
            };
        }
        if frozen {
            return SanitizeCaps {
                supported: false,
                method: Some(SanitizeAction::AtaSecureErase.label().into()),
                action: None,
                detail: format!(
                    "{} ({model}) has a frozen ATA security lock. A firmware erase cannot be issued until the drive is power-cycled (and even then, many systems re-freeze it). Use overwrite mode, or a vendor tool after a supported unfreeze.",
                    snap.identity.display_name
                ),
            };
        }
        if enabled {
            return SanitizeCaps {
                supported: false,
                method: Some(SanitizeAction::AtaSecureErase.label().into()),
                action: None,
                detail: format!(
                    "{} ({model}) already has an ATA security password set. This tool will not guess or replace that password.",
                    snap.identity.display_name
                ),
            };
        }
        SanitizeCaps {
            supported: true,
            method: Some(SanitizeAction::AtaSecureErase.label().into()),
            action: Some(SanitizeAction::AtaSecureErase),
            detail: format!(
                "ATA Secure Erase is available on {} ({model}). This tool will set a temporary empty user password and issue SECURITY ERASE UNIT. A typed confirmation of the device name is required. A power loss mid-erase can leave the drive locked. This tool is not NIST certified.",
                snap.identity.display_name
            ),
        }
    }

    fn ata_identify(path: &Path) -> Result<[u16; 256], String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| format!("open {}: {e}", path.display()))?;
        let mut data = [0u8; 512];
        let mut sense = [0u8; 32];
        // ATA PASS-THROUGH (16): IDENTIFY DEVICE (0xEC), protocol PIO Data-In (4), t_dir=1.
        let mut cdb = [0u8; 16];
        cdb[0] = 0x85;
        cdb[1] = 4 << 1; // PROTOCOL = PIO Data-In
        cdb[2] = (1 << 3) | (1 << 2) | 0x2; // t_dir=1, byte_block=1, t_length=02 (sector count)
        cdb[6] = 1; // sector count
        cdb[14] = 0xEC; // IDENTIFY DEVICE
        let mut hdr = sg_hdr(
            SG_DXFER_FROM_DEV,
            data.as_mut_ptr(),
            512,
            cdb.as_mut_ptr(),
            16,
            sense.as_mut_ptr(),
            32,
            10_000,
        );
        let rc = unsafe { libc::ioctl(file.as_raw_fd(), SG_IO, &mut hdr as *mut _) };
        if rc < 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        if hdr.status != 0 || hdr.host_status != 0 || hdr.driver_status != 0 {
            return Err(format!(
                "SG_IO IDENTIFY status={} host={} driver={}",
                hdr.status, hdr.host_status, hdr.driver_status
            ));
        }
        let mut words = [0u16; 256];
        for i in 0..256 {
            words[i] = u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
        }
        Ok(words)
    }

    fn issue_ata(snap: &TargetSnapshot) -> Result<SanitizeOutcome, Error> {
        let path = &snap.identity.path;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| Error::Open {
                path: path.clone(),
                detail: format!("open for ATA secure erase failed: {e}"),
            })?;

        // SECURITY SET PASSWORD (0xF1) with an empty user password, then
        // SECURITY ERASE UNIT (0xF4) using that same password.
        ata_security_cmd(file.as_raw_fd(), 0xF1, true, 15_000).map_err(|e| {
            Error::Message(format!(
                "ATA SECURITY SET PASSWORD failed on {}: {e}",
                path.display()
            ))
        })?;
        ata_security_cmd(file.as_raw_fd(), 0xF4, true, 0).map_err(|e| {
            Error::Message(format!(
                "ATA SECURITY ERASE UNIT failed on {}: {e}. The drive may now have a security password set.",
                path.display()
            ))
        })?;
        Ok(SanitizeOutcome {
            method: SanitizeAction::AtaSecureErase.label().into(),
            detail: format!("SECURITY ERASE UNIT completed on {}", path.display()),
        })
    }

    fn ata_security_cmd(
        fd: i32,
        command: u8,
        with_data: bool,
        timeout_ms: u32,
    ) -> Result<(), String> {
        // 512-byte security payload: word 0 identifier/level, words 1..16 password (zeros = empty).
        let mut data = [0u8; 512];
        // identifier = user (bit 0 = 0), level = high (bit 8 of word 0 = 1) => bytes 0-1 = 0x00, 0x01
        data[1] = 0x01;
        let mut sense = [0u8; 32];
        let mut cdb = [0u8; 16];
        cdb[0] = 0x85; // ATA PASS-THROUGH (16)
        if with_data {
            cdb[1] = 5 << 1; // PROTOCOL = PIO Data-Out
            cdb[2] = (1 << 2) | 0x2; // t_dir=0, byte_block=1, t_length=02
            cdb[6] = 1; // sector count
        } else {
            cdb[1] = 3 << 1; // PROTOCOL = Non-data
            cdb[2] = 0;
        }
        cdb[14] = command;
        let mut hdr = if with_data {
            sg_hdr(
                SG_DXFER_TO_DEV,
                data.as_mut_ptr(),
                512,
                cdb.as_mut_ptr(),
                16,
                sense.as_mut_ptr(),
                32,
                timeout_ms,
            )
        } else {
            sg_hdr(
                SG_DXFER_NONE,
                std::ptr::null_mut(),
                0,
                cdb.as_mut_ptr(),
                16,
                sense.as_mut_ptr(),
                32,
                timeout_ms,
            )
        };
        let rc = unsafe { libc::ioctl(fd, SG_IO, &mut hdr as *mut _) };
        if rc < 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        if hdr.status != 0 || hdr.host_status != 0 || hdr.driver_status != 0 {
            return Err(format!(
                "SG_IO command 0x{command:02x} status={} host={} driver={}",
                hdr.status, hdr.host_status, hdr.driver_status
            ));
        }
        let _ = ErrorKind::Other;
        Ok(())
    }

    fn sg_hdr(
        dir: i32,
        dxferp: *mut u8,
        dxfer_len: u32,
        cmdp: *mut u8,
        cmd_len: u8,
        sbp: *mut u8,
        mx_sb_len: u8,
        timeout: u32,
    ) -> SgIoHdr {
        SgIoHdr {
            interface_id: i32::from(b'S'),
            dxfer_direction: dir,
            cmd_len,
            mx_sb_len,
            iovec_count: 0,
            dxfer_len,
            dxferp,
            cmdp,
            sbp,
            timeout,
            flags: 0,
            pack_id: 0,
            usr_ptr: std::ptr::null_mut(),
            status: 0,
            masked_status: 0,
            msg_status: 0,
            sb_len_wr: 0,
            host_status: 0,
            driver_status: 0,
            resid: 0,
            duration: 0,
            info: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;
    use crate::method::Method;
    use crate::planner::{MediaKind, Plan, Scope, TargetSnapshot};
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::sync::Mutex;

    struct MockBackend {
        caps: SanitizeCaps,
        issued: Mutex<Vec<SanitizeAction>>,
        fail: bool,
    }

    impl Backend for MockBackend {
        fn caps(&self, _snap: &TargetSnapshot) -> SanitizeCaps {
            self.caps.clone()
        }
        fn issue(
            &self,
            _snap: &TargetSnapshot,
            action: SanitizeAction,
        ) -> Result<SanitizeOutcome, Error> {
            self.issued.lock().unwrap().push(action);
            if self.fail {
                return Err(Error::Message("mock issue failed".into()));
            }
            Ok(SanitizeOutcome {
                method: action.label().into(),
                detail: "mock issued".into(),
            })
        }
    }

    fn nvme_snap() -> TargetSnapshot {
        TargetSnapshot {
            identity: DeviceIdentity {
                path: PathBuf::from("/dev/nvme1n1"),
                canonical_path: PathBuf::from("/dev/nvme1n1"),
                display_name: "nvme1n1".into(),
                model: Some("Mock NVMe".into()),
                serial: Some("MOCKSERIAL".into()),
                wwn: None,
                size: 1_000_000_000,
                file_id: Some((1, 1)),
                media: "nvme".into(),
            },
            kind: MediaKind::Nvme,
            is_system: false,
            is_app_disk: false,
            mounts: vec![],
            in_use: false,
            is_raid_member: false,
            is_lvm_member: false,
            backing_file: None,
            is_symlink: false,
            is_regular_file: false,
            is_directory: false,
            is_block_device: true,
            logical_block_size: 512,
        }
    }

    fn plan_for(snap: &TargetSnapshot) -> Plan {
        let flags = SafetyFlags {
            typed_confirm: Some(snap.identity.display_name.clone()),
            explicit_target: true,
            countdown_secs: 5,
            ..SafetyFlags::default()
        };
        plan(snap, Scope::FirmwareSanitize, Method::zeros(), None, &flags).expect("plan")
    }

    fn supported_caps() -> SanitizeCaps {
        SanitizeCaps {
            supported: true,
            method: Some(SanitizeAction::NvmeSanitizeCrypto.label().into()),
            action: Some(SanitizeAction::NvmeSanitizeCrypto),
            detail: "mock supports crypto erase".into(),
        }
    }

    #[test]
    fn platform_backend_issue_refuses_when_forbid_env_is_set() {
        assert!(firmware_commands_forbidden());
        let snap = nvme_snap();
        let err = PlatformBackend
            .issue(&snap, SanitizeAction::NvmeSanitizeCrypto)
            .expect_err("must refuse");
        assert!(matches!(err, Error::FirmwareNotSupported(_)));
    }

    #[test]
    fn mock_issues_when_supported() {
        let snap = nvme_snap();
        let planned = plan_for(&snap);
        let backend = MockBackend {
            caps: supported_caps(),
            issued: Mutex::new(Vec::new()),
            fail: false,
        };
        let report = finish_sanitize(&backend, &snap, planned, &Arc::new(AtomicBool::new(false)))
            .expect("mock issue");
        assert_eq!(report.outcome, Outcome::Success);
        assert_eq!(
            *backend.issued.lock().unwrap(),
            vec![SanitizeAction::NvmeSanitizeCrypto]
        );
    }

    #[test]
    fn mock_does_not_issue_when_caps_say_unsupported() {
        let snap = nvme_snap();
        let planned = plan_for(&snap);
        let backend = MockBackend {
            caps: SanitizeCaps {
                supported: false,
                method: None,
                action: None,
                detail: "not supported on this mock".into(),
            },
            issued: Mutex::new(Vec::new()),
            fail: false,
        };
        let err = finish_sanitize(&backend, &snap, planned, &Arc::new(AtomicBool::new(false)))
            .expect_err("must refuse");
        assert!(matches!(err, Error::FirmwareNotSupported(_)));
        assert!(backend.issued.lock().unwrap().is_empty());
    }

    #[test]
    fn mock_does_not_issue_when_cancelled() {
        let snap = nvme_snap();
        let planned = plan_for(&snap);
        let backend = MockBackend {
            caps: supported_caps(),
            issued: Mutex::new(Vec::new()),
            fail: false,
        };
        let cancel = Arc::new(AtomicBool::new(true));
        let err = finish_sanitize(&backend, &snap, planned, &cancel).expect_err("cancel");
        assert!(matches!(err, Error::Cancelled));
        assert!(backend.issued.lock().unwrap().is_empty());
    }

    #[test]
    fn dry_run_does_not_issue() {
        let snap = nvme_snap();
        let mut planned = plan_for(&snap);
        planned.dry_run = true;
        let backend = MockBackend {
            caps: supported_caps(),
            issued: Mutex::new(Vec::new()),
            fail: false,
        };
        let report = finish_sanitize(&backend, &snap, planned, &Arc::new(AtomicBool::new(false)))
            .expect("dry run");
        assert_eq!(report.outcome, Outcome::DryRun);
        assert!(backend.issued.lock().unwrap().is_empty());
    }

    #[test]
    fn detect_rejects_image_files() {
        let mut snap = nvme_snap();
        snap.is_regular_file = true;
        snap.is_block_device = false;
        snap.kind = MediaKind::Image;
        let caps = detect(&snap);
        assert!(!caps.supported);
    }
}
