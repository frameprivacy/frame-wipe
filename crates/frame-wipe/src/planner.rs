//! Pure planner: given a snapshot and flags, either produce a plan or a refusal.
//! Property tests cover every refusal path without touching a real disk.

use crate::error::{Error, Guard};
use crate::identity::{confirmation_matches, DeviceIdentity};
use crate::method::{Method, Pass};
use crate::safety::SafetyFlags;
use crate::units::align_down;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    WholeDevice,
    Partition,
    FreeSpace,
    Files,
    FirmwareSanitize,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Self::WholeDevice => "whole device",
            Self::Partition => "partition",
            Self::FreeSpace => "free space",
            Self::Files => "files",
            Self::FirmwareSanitize => "firmware sanitize",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Hdd,
    Ssd,
    Nvme,
    Optical,
    Ram,
    Loop,
    Image,
    Virtual,
    Unknown,
}

impl MediaKind {
    pub fn is_virtual(self) -> bool {
        matches!(self, Self::Loop | Self::Image | Self::Virtual | Self::Ram)
    }

    /// Short label for the UI (HDD, SSD, NVMe, ...).
    pub fn label(self) -> &'static str {
        match self {
            Self::Hdd => "HDD",
            Self::Ssd => "SSD",
            Self::Nvme => "NVMe",
            Self::Optical => "Optical",
            Self::Ram => "RAM",
            Self::Loop => "Loop",
            Self::Image => "Image",
            Self::Virtual => "Virtual",
            Self::Unknown => "Unknown",
        }
    }
}

/// A point-in-time view of a target, used by the planner and re-checked later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetSnapshot {
    pub identity: DeviceIdentity,
    pub kind: MediaKind,
    pub is_system: bool,
    pub is_app_disk: bool,
    pub mounts: Vec<String>,
    pub in_use: bool,
    pub is_raid_member: bool,
    pub is_lvm_member: bool,
    pub backing_file: Option<PathBuf>,
    pub is_symlink: bool,
    pub is_regular_file: bool,
    pub is_directory: bool,
    pub is_block_device: bool,
    pub logical_block_size: u64,
}

impl TargetSnapshot {
    pub fn is_virtual(&self) -> bool {
        self.kind.is_virtual() || self.backing_file.is_some() || self.is_regular_file
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub identity: DeviceIdentity,
    pub scope: Scope,
    pub method: Method,
    pub passes: Vec<Pass>,
    pub chunk_size: u64,
    pub size: u64,
    pub dry_run: bool,
    pub skip_bad_sectors: bool,
    pub countdown_secs: u32,
    pub virtual_or_image: bool,
    /// Notes the operator should read before confirming (TRIM limits, etc.).
    pub warnings: Vec<String>,
}

const DEFAULT_CHUNK: u64 = 8 * 1024 * 1024;
const MIN_CHUNK: u64 = 4096;
const MAX_CHUNK: u64 = 128 * 1024 * 1024;

pub fn normalize_chunk_size(requested: Option<u64>, block_size: u64) -> u64 {
    let block = if block_size == 0 { 512 } else { block_size };
    let mut chunk = requested.unwrap_or(DEFAULT_CHUNK);
    if chunk < MIN_CHUNK {
        chunk = MIN_CHUNK;
    }
    if chunk > MAX_CHUNK {
        chunk = MAX_CHUNK;
    }
    let aligned = align_down(chunk, block);
    if aligned < block {
        block
    } else {
        aligned
    }
}

/// Plan a wipe, or return a refusal. This function is pure: no I/O, no unwrap.
pub fn plan(
    target: &TargetSnapshot,
    scope: Scope,
    method: Method,
    chunk_size: Option<u64>,
    flags: &SafetyFlags,
) -> Result<Plan, Error> {
    if target.is_symlink && scope == Scope::Files {
        return Err(Error::Symlink(target.identity.path.clone()));
    }
    if target.is_symlink && scope != Scope::Files {
        return Err(Error::refused(
            Guard::FollowedLink,
            format!(
                "refusing to follow symlink {}",
                target.identity.path.display()
            ),
        ));
    }

    if !flags.explicit_target {
        return Err(Error::refused(
            Guard::YesWithoutNamedTarget,
            "the target must be named explicitly (a path or device name on the command line)",
        ));
    }

    if flags.assume_yes && !flags.explicit_target {
        return Err(Error::refused(
            Guard::YesWithoutNamedTarget,
            "--yes requires the device or path to be named explicitly",
        ));
    }

    // Typed confirmation of the device name is required for every destructive scope
    // except a dry run of an already-named regular file, which still needs --yes or --confirm.
    let needs_confirm = !flags.dry_run || target.is_block_device;
    if needs_confirm {
        match flags.typed_confirm.as_deref() {
            None => {
                // Regular image files may proceed with --yes alone, since the path is the name.
                let image_yes = flags.assume_yes
                    && target.is_regular_file
                    && !target.is_block_device
                    && scope != Scope::FirmwareSanitize;
                if !image_yes {
                    return Err(crate::safety::confirmation_required_error(&target.identity));
                }
            }
            Some(typed) => {
                if !confirmation_matches(&target.identity.display_name, typed) {
                    return Err(crate::safety::confirmation_mismatch_error(
                        &target.identity,
                        typed,
                    ));
                }
            }
        }
    }

    if target.is_system
        && (scope == Scope::WholeDevice
            || scope == Scope::Partition
            || scope == Scope::FirmwareSanitize
            || scope == Scope::FreeSpace)
    {
        if !flags.allow_system_disk {
            return Err(Error::refused(
                Guard::SystemDisk,
                format!(
                    "{} is the boot or system disk (model {}, serial {}). Refusing to wipe it. If you really mean it, pass --allow-system-disk and type the device name.",
                    target.identity.display_name,
                    target.identity.model.as_deref().unwrap_or("unknown"),
                    target.identity.serial.as_deref().unwrap_or("unknown"),
                ),
            ));
        }
    }

    if target.is_app_disk
        && matches!(
            scope,
            Scope::WholeDevice | Scope::Partition | Scope::FirmwareSanitize
        )
    {
        if !flags.allow_app_disk {
            return Err(Error::refused(
                Guard::AppDisk,
                format!(
                    "{} is the disk Frame Wipe itself is running from. Refusing to wipe it (pass --allow-app-disk only if you understand the process will likely crash mid-wipe).",
                    target.identity.display_name
                ),
            ));
        }
    }

    if !target.mounts.is_empty()
        && scope != Scope::FreeSpace
        && scope != Scope::Files
        && !flags.allow_mounted
    {
        return Err(Error::refused(
            Guard::Mounted,
            format!(
                "{} is mounted at {}. Unmount it first, or use free-space mode. (--allow-mounted overrides this, and is dangerous.)",
                target.identity.display_name,
                target.mounts.join(", ")
            ),
        ));
    }

    if target.in_use && scope != Scope::FreeSpace && scope != Scope::Files && !flags.allow_in_use {
        return Err(Error::refused(
            Guard::InUse,
            format!(
                "{} is in use (open files, swap, or an exclusive lock failed). Close users of the volume first, or pass --allow-in-use.",
                target.identity.display_name
            ),
        ));
    }

    if target.is_raid_member && !flags.allow_raid {
        return Err(Error::refused(
            Guard::RaidMember,
            format!(
                "{} is a RAID member. Wiping one leg of an array can take the array down and is easy to mis-target. Pass --allow-raid only if you mean to wipe this member.",
                target.identity.display_name
            ),
        ));
    }

    if target.is_lvm_member && !flags.allow_raid {
        return Err(Error::refused(
            Guard::LvmMember,
            format!(
                "{} is an LVM or device-mapper member. Pass --allow-raid if you mean to wipe this physical volume (the same flag covers RAID and LVM members).",
                target.identity.display_name
            ),
        ));
    }

    if !target.identity.is_identified() && !flags.allow_unidentified && !target.is_regular_file {
        return Err(crate::identity::missing_identity_error(&target.identity));
    }

    if scope == Scope::FirmwareSanitize && target.is_regular_file {
        return Err(Error::FirmwareNotSupported(
            "firmware sanitize is not available on disk image files".into(),
        ));
    }

    if scope == Scope::FreeSpace && target.mounts.is_empty() && !target.is_directory {
        return Err(Error::refused(
            Guard::UnmountRequired,
            "free-space mode needs a mounted volume or a directory on that volume",
        ));
    }

    let virtual_or_image = target.is_virtual();
    let countdown_secs = flags.countdown_for(virtual_or_image)?;
    let chunk = normalize_chunk_size(chunk_size, target.logical_block_size);

    let mut warnings = Vec::new();
    if matches!(target.kind, MediaKind::Ssd | MediaKind::Nvme) {
        warnings.push(
            "This looks like an SSD or NVMe device. An overwrite cannot reliably reach every overprovisioned or remapped block. Prefer the firmware sanitize command when the drive supports it, and treat overwrite as NIST SP 800-88 Clear, not Purge. This tool is not certified.".into(),
        );
    }
    if flags.skip_bad_sectors {
        warnings
            .push("Bad sectors will be skipped. Those ranges may still hold leftover data.".into());
    }
    if flags.allow_system_disk || flags.allow_app_disk || flags.allow_mounted || flags.allow_raid {
        warnings.push("One or more safety overrides are in effect.".into());
    }

    Ok(Plan {
        identity: target.identity.clone(),
        scope,
        method: method.clone(),
        passes: method.passes(),
        chunk_size: chunk,
        size: target.identity.size,
        dry_run: flags.dry_run,
        skip_bad_sectors: flags.skip_bad_sectors,
        countdown_secs,
        virtual_or_image,
        warnings,
    })
}
