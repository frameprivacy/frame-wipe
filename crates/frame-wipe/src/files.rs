//! Overwrite regular files and directory trees. Never follows symlinks.

use crate::engine::{wipe, WipeRequest};
use crate::error::{Error, Guard};
use crate::method::Method;
use crate::planner::Scope;
use crate::report::Report;
use crate::safety::SafetyFlags;
use std::fs::{self, symlink_metadata};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Walk `root` without following symlinks and wipe each regular file.
pub fn wipe_tree(
    root: &Path,
    method: Method,
    chunk_size: Option<u64>,
    flags: SafetyFlags,
    cancel: &Arc<AtomicBool>,
    mut on_file: impl FnMut(&Path),
) -> Result<Vec<Report>, Error> {
    let meta = symlink_metadata(root).map_err(|e| Error::Open {
        path: root.to_path_buf(),
        detail: e.to_string(),
    })?;
    if meta.file_type().is_symlink() {
        return Err(Error::Symlink(root.to_path_buf()));
    }
    if crate::safety::is_current_exe(root) {
        return Err(Error::refused(
            Guard::AppDisk,
            format!(
                "refusing to overwrite the running executable {}",
                root.display()
            ),
        ));
    }
    let mut reports = Vec::new();
    if meta.is_file() {
        on_file(root);
        reports.push(wipe_one_file(root, method, chunk_size, flags, cancel)?);
        return Ok(reports);
    }
    if !meta.is_dir() {
        return Err(Error::InvalidTarget(root.to_path_buf()));
    }
    if !flags.assume_yes && flags.typed_confirm.is_none() {
        return Err(crate::safety::confirmation_required_error(
            &crate::device::inspect(root)?.identity,
        ));
    }
    let start_dev = file_dev(root, &meta);
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(|e| Error::Open {
            path: dir.clone(),
            detail: e.to_string(),
        })?;
        for ent in entries {
            let ent = ent.map_err(|e| Error::Open {
                path: dir.clone(),
                detail: e.to_string(),
            })?;
            let path = ent.path();
            let meta = symlink_metadata(&path).map_err(|e| Error::Open {
                path: path.clone(),
                detail: e.to_string(),
            })?;
            if meta.file_type().is_symlink() {
                return Err(Error::Symlink(path));
            }
            if file_dev(&path, &meta) != start_dev {
                // Do not cross filesystem / mount boundaries.
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
                continue;
            }
            if !meta.is_file() {
                return Err(Error::InvalidTarget(path));
            }
            if crate::safety::is_current_exe(&path) {
                return Err(Error::refused(
                    Guard::AppDisk,
                    format!(
                        "refusing to overwrite the running executable {}",
                        path.display()
                    ),
                ));
            }
            on_file(&path);
            let mut file_flags = flags.clone();
            file_flags.explicit_target = true;
            // After the operator confirmed the tree root, child files may
            // proceed with --yes and their own file name as confirmation.
            if flags.assume_yes || flags.typed_confirm.is_some() {
                file_flags.typed_confirm = Some(crate::device::file_display_name(&path));
                file_flags.assume_yes = true;
            }
            reports.push(wipe_one_file(
                &path,
                method.clone(),
                chunk_size,
                file_flags,
                cancel,
            )?);
        }
    }
    Ok(reports)
}

fn wipe_one_file(
    path: &Path,
    method: Method,
    chunk_size: Option<u64>,
    mut flags: SafetyFlags,
    cancel: &Arc<AtomicBool>,
) -> Result<Report, Error> {
    flags.explicit_target = true;
    if flags.typed_confirm.is_none() && flags.assume_yes {
        flags.typed_confirm = Some(crate::device::file_display_name(path));
    }
    // Directory walks already confirmed at the root; individual files use a 0s countdown
    // only because they are regular files (planner allows this for virtual_or_image).
    let req = WipeRequest {
        path: path.to_path_buf(),
        scope: Scope::Files,
        method,
        chunk_size,
        flags,
    };
    wipe(&req, cancel, |_| {})
}

#[cfg(unix)]
fn file_dev(_path: &std::path::Path, meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.dev()
}

#[cfg(windows)]
fn file_dev(path: &std::path::Path, _meta: &std::fs::Metadata) -> u64 {
    crate::device::win_meta::volume_serial(path).unwrap_or(0)
}

#[cfg(not(any(unix, windows)))]
fn file_dev(_path: &std::path::Path, _meta: &std::fs::Metadata) -> u64 {
    0
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::method::Method;
    use crate::safety::SafetyFlags;
    use std::os::unix::io::AsRawFd;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn refuses_locked_file_and_leaves_content() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("FRAME_WIPE_FORBID_REAL_DISKS", "1");
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("FRAME_WIPE_TEST_DIR", dir.path());
        let path = dir.path().join("locked.txt");
        let original = b"keep-me-please!!!";
        fs::write(&path, original).unwrap();
        let holder = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let rc = unsafe { libc::flock(holder.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        assert_eq!(rc, 0, "failed to lock test file");
        let flags = SafetyFlags {
            explicit_target: true,
            assume_yes: true,
            typed_confirm: Some("locked.txt".into()),
            countdown_secs: 0,
            ..SafetyFlags::default()
        };
        let err = wipe_tree(
            &path,
            Method::zeros(),
            None,
            flags,
            &Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(err.guard(), Some(Guard::InUse), "{err}");
        assert_eq!(fs::read(&path).unwrap(), original);
        drop(holder);
    }
}
