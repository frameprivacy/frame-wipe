//! Safety flags, restricted test mode, and refusal checks shared by the planner and the engine.

use crate::error::{Error, Guard};
use crate::identity::DeviceIdentity;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Default countdown, in seconds, before the first write.
pub const DEFAULT_COUNTDOWN_SECS: u32 = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyFlags {
    pub allow_system_disk: bool,
    pub allow_app_disk: bool,
    pub allow_mounted: bool,
    pub allow_raid: bool,
    pub allow_in_use: bool,
    pub allow_unidentified: bool,
    pub assume_yes: bool,
    pub typed_confirm: Option<String>,
    /// True when the operator named the target on the command line (not inferred).
    pub explicit_target: bool,
    pub countdown_secs: u32,
    pub dry_run: bool,
    pub skip_bad_sectors: bool,
    pub resume: bool,
}

impl Default for SafetyFlags {
    fn default() -> Self {
        Self {
            allow_system_disk: false,
            allow_app_disk: false,
            allow_mounted: false,
            allow_raid: false,
            allow_in_use: false,
            allow_unidentified: false,
            assume_yes: false,
            typed_confirm: None,
            explicit_target: false,
            countdown_secs: DEFAULT_COUNTDOWN_SECS,
            dry_run: false,
            skip_bad_sectors: false,
            resume: false,
        }
    }
}

impl SafetyFlags {
    /// `--countdown 0` is only legal for image files and verified virtual devices.
    pub fn countdown_for(&self, virtual_or_image: bool) -> Result<u32, Error> {
        if self.countdown_secs == 0 && !virtual_or_image {
            return Err(Error::refused(
                Guard::ConfirmationMissing,
                "a zero-second countdown is only allowed for disk images and verified virtual devices, not physical disks",
            ));
        }
        Ok(self.countdown_secs)
    }
}

/// True when the process must refuse real physical disks (tests and CI).
pub fn real_disks_forbidden() -> bool {
    if cfg!(test) {
        return true;
    }
    match std::env::var("FRAME_WIPE_FORBID_REAL_DISKS") {
        Ok(v) => matches!(v.as_str(), "1" | "true" | "yes" | "TRUE" | "YES"),
        Err(_) => std::env::var("FRAME_WIPE_TEST").ok().as_deref() == Some("1"),
    }
}

pub fn test_dir() -> Option<PathBuf> {
    std::env::var_os("FRAME_WIPE_TEST_DIR").map(PathBuf::from)
}

/// A path is a safe test target if it is a regular file (or will be) under the temp/test dir,
/// never a raw physical device path.
pub fn path_looks_like_raw_device(path: &Path) -> bool {
    let s = path.to_string_lossy();
    if cfg!(windows) {
        let l = s.to_ascii_lowercase();
        return l.contains(r"\physicaldrive") || l.starts_with(r"\\.\") || l.starts_with(r"\\?\");
    }
    if s.starts_with("/dev/") {
        let name = s.trim_start_matches("/dev/").trim_start_matches('r');
        // Permit explicitly checked loop/ram/fd later; this is a first-pass filter.
        if name.starts_with("loop") || name.starts_with("ram") || name.starts_with("fd") {
            return false;
        }
        return true;
    }
    false
}

/// In restricted mode, only regular files under an allowed directory, or virtual devices
/// whose backing file sits in that directory, may be written.
pub fn assert_permitted_in_restricted_mode(
    path: &Path,
    backing_file: Option<&Path>,
    is_virtual: bool,
) -> Result<(), Error> {
    if !real_disks_forbidden() {
        return Ok(());
    }
    if path_looks_like_raw_device(path) && !is_virtual {
        return Err(Error::refused(
            Guard::RestrictedTestTarget,
            format!(
                "refusing to write to raw device {} while FRAME_WIPE_FORBID_REAL_DISKS is set",
                path.display()
            ),
        ));
    }
    if is_virtual {
        let Some(backing) = backing_file else {
            return Err(Error::refused(
                Guard::RestrictedTestTarget,
                format!(
                    "virtual device {} has no backing file; refusing in restricted mode",
                    path.display()
                ),
            ));
        };
        return assert_under_test_dir(backing);
    }
    assert_not_raw_and_regular_or_under_dir(path)
}

fn assert_under_test_dir(path: &Path) -> Result<(), Error> {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if path_looks_like_raw_device(&canon) {
        return Err(Error::refused(
            Guard::RestrictedTestTarget,
            format!("backing file looks like a raw device: {}", canon.display()),
        ));
    }
    if let Some(dir) = test_dir() {
        let dir = dir.canonicalize().unwrap_or(dir);
        if !canon.starts_with(&dir) {
            return Err(Error::refused(
                Guard::RestrictedTestTarget,
                format!(
                    "{} is not under the test directory {}",
                    canon.display(),
                    dir.display()
                ),
            ));
        }
    }
    Ok(())
}

fn assert_not_raw_and_regular_or_under_dir(path: &Path) -> Result<(), Error> {
    if path_looks_like_raw_device(path) {
        return Err(Error::refused(
            Guard::RestrictedTestTarget,
            format!(
                "raw device path {} is forbidden in restricted mode",
                path.display()
            ),
        ));
    }
    if path.exists() {
        let meta = std::fs::symlink_metadata(path).map_err(Error::from)?;
        if meta.file_type().is_symlink() {
            return Err(Error::Symlink(path.to_path_buf()));
        }
        if !meta.file_type().is_file() {
            return Err(Error::refused(
                Guard::RestrictedTestTarget,
                format!(
                    "{} is not a regular file; refusing in restricted mode",
                    path.display()
                ),
            ));
        }
    }
    if let Some(dir) = test_dir() {
        let dir = dir.canonicalize().unwrap_or(dir);
        let parent = path.parent().unwrap_or(path);
        let parent = parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf());
        if !parent.starts_with(&dir) && !path.starts_with(&dir) {
            // Also allow the system temp dir used by tempfile.
            if !is_temp_like(&parent) {
                return Err(Error::refused(
                    Guard::RestrictedTestTarget,
                    format!("{} is not under the test directory", path.display()),
                ));
            }
        }
    }
    Ok(())
}

fn is_temp_like(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("/tmp")
        || s.contains("/var/folders/")
        || s.contains(r"\Temp")
        || s.contains(r"\AppData\Local\Temp")
        || std::env::temp_dir()
            .canonicalize()
            .ok()
            .map(|t| path.starts_with(t))
            .unwrap_or(false)
}

/// Refuse to wipe the running executable itself.
pub fn is_current_exe(path: &Path) -> bool {
    match (std::env::current_exe(), path.canonicalize()) {
        (Ok(exe), Ok(canon)) => {
            if let Ok(exe_c) = exe.canonicalize() {
                return exe_c == canon;
            }
            exe == canon
        }
        _ => false,
    }
}

pub fn confirmation_required_error(identity: &DeviceIdentity) -> Error {
    Error::refused(
        Guard::ConfirmationMissing,
        format!(
            "type the device name '{}' to confirm (pass --confirm {})",
            identity.display_name, identity.display_name
        ),
    )
}

pub fn confirmation_mismatch_error(identity: &DeviceIdentity, typed: &str) -> Error {
    Error::refused(
        Guard::ConfirmationMismatch,
        format!(
            "confirmation '{typed}' does not match device name '{}'",
            identity.display_name
        ),
    )
}
