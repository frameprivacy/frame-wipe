//! Fill a volume's free space with the chosen pattern, then delete the filler files.
//! Stays on a single mount. Does not follow symlinks.

use crate::error::Error;
use crate::method::{Method, Pattern};
use crate::open::{alloc_aligned, fill_pattern};
use crate::planner::{plan, Scope};
use crate::progress::{Phase, Progress};
use crate::report::{Outcome, Report};
use crate::safety::SafetyFlags;
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use time::OffsetDateTime;

const FILL_DIR: &str = ".framewipe-free";
const FILE_SIZE: u64 = 64 * 1024 * 1024;

#[cfg(test)]
static TEST_FILE_CAP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn file_cap() -> u64 {
    #[cfg(test)]
    {
        let n = TEST_FILE_CAP.load(Ordering::SeqCst);
        if n > 0 {
            return n;
        }
    }
    FILE_SIZE
}

pub fn wipe_free_space(
    mount: &Path,
    method: Method,
    flags: SafetyFlags,
    cancel: &Arc<AtomicBool>,
    mut on_progress: impl FnMut(Progress),
) -> Result<Report, Error> {
    let snap = crate::device::inspect(mount)?;
    let planned = plan(&snap, Scope::FreeSpace, method.clone(), None, &flags)?;
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
    crate::confirm::countdown(planned.countdown_secs, cancel, |_| Ok(()))?;

    let meta = fs::symlink_metadata(mount).map_err(|e| Error::Open {
        path: mount.to_path_buf(),
        detail: e.to_string(),
    })?;
    if meta.file_type().is_symlink() {
        return Err(Error::Symlink(mount.to_path_buf()));
    }
    if !meta.is_dir() {
        return Err(Error::InvalidTarget(mount.to_path_buf()));
    }

    let dir = mount.join(FILL_DIR);
    if dir.exists() {
        if fs::symlink_metadata(&dir)?.file_type().is_symlink() {
            return Err(Error::Symlink(dir));
        }
    } else {
        fs::create_dir(&dir).map_err(|e| Error::Open {
            path: dir.clone(),
            detail: e.to_string(),
        })?;
    }

    let pattern = method
        .passes()
        .first()
        .map(|p| p.pattern)
        .unwrap_or(Pattern::Zeros);
    let seed = {
        let mut s = [0u8; 32];
        let _ = getrandom::getrandom(&mut s);
        s
    };
    let mut rng = if pattern == Pattern::Random {
        Some(ChaCha20Rng::from_seed(seed))
    } else {
        None
    };

    let mut bytes = 0u64;
    let mut index = 0u32;
    let mut files: Vec<PathBuf> = Vec::new();
    let started = OffsetDateTime::now_utc();

    let result = (|| -> Result<(), Error> {
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(Error::Cancelled);
            }
            let path = dir.join(format!("fill-{index:06}"));
            index += 1;
            // Record the path before writing so cancel or I/O errors mid-fill
            // still clean the file up.
            files.push(path.clone());
            match fill_one(&path, file_cap(), pattern, &mut rng, cancel, &mut bytes) {
                Ok(true) => {
                    on_progress(Progress {
                        phase: Phase::Writing,
                        pass: 1,
                        passes: 1,
                        bytes_done: bytes,
                        bytes_total: 0,
                        throughput_bps: 0,
                        eta_secs: None,
                        countdown_remaining: None,
                        message: None,
                    });
                }
                Ok(false) => break, // no space left
                Err(e) => return Err(e),
            }
        }
        Ok(())
    })();

    // Always try to remove filler files, even on cancel or error.
    let mut cleanup_err: Option<Error> = None;
    for p in files.iter().rev() {
        if let Err(e) = fs::remove_file(p) {
            if e.kind() != std::io::ErrorKind::NotFound {
                cleanup_err = Some(Error::Message(format!(
                    "failed to remove {}: {e}",
                    p.display()
                )));
            }
        }
    }
    // Sweep anything still in the filler directory (for example a file
    // created just as we were cancelled, or a name we failed to record).
    if let Ok(entries) = fs::read_dir(&dir) {
        for ent in entries.flatten() {
            let p = ent.path();
            if p.is_file() {
                if let Err(e) = fs::remove_file(&p) {
                    if e.kind() != std::io::ErrorKind::NotFound && cleanup_err.is_none() {
                        cleanup_err = Some(Error::Message(format!(
                            "failed to remove {}: {e}",
                            p.display()
                        )));
                    }
                }
            }
        }
    }
    if let Err(e) = fs::remove_dir(&dir) {
        // Directory may not be empty if a file removal failed.
        if cleanup_err.is_none() {
            cleanup_err = Some(Error::Message(format!(
                "failed to remove {}: {e}",
                dir.display()
            )));
        }
    }
    // Best-effort sync of the directory.
    if let Ok(f) = File::open(mount) {
        let _ = f.sync_all();
    }

    match result {
        Err(e) => {
            let _ = cleanup_err;
            Err(e)
        }
        Ok(()) => {
            if let Some(e) = cleanup_err {
                return Err(e);
            }
            let finished = OffsetDateTime::now_utc();
            Ok(crate::report::from_plan(
                &planned,
                Outcome::Success,
                started,
                finished,
                bytes,
                Vec::new(),
                None,
            ))
        }
    }
}

/// Returns Ok(true) if a file was written, Ok(false) if the volume is full.
fn fill_one(
    path: &Path,
    cap: u64,
    pattern: Pattern,
    rng: &mut Option<ChaCha20Rng>,
    cancel: &Arc<AtomicBool>,
    bytes: &mut u64,
) -> Result<bool, Error> {
    let mut file = match OpenOptions::new().create_new(true).write(true).open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(Error::Open {
                path: path.to_path_buf(),
                detail: e.to_string(),
            });
        }
        Err(e) if is_no_space(&e) => {
            // ENOSPC
            return Ok(false);
        }
        Err(e) => {
            return Err(Error::Open {
                path: path.to_path_buf(),
                detail: e.to_string(),
            });
        }
    };
    let mut buf = alloc_aligned(1024 * 1024, 4096)?;
    let mut written = 0u64;
    while written < cap {
        if cancel.load(Ordering::SeqCst) {
            let _ = file.sync_all();
            return Err(Error::Cancelled);
        }
        let n = ((cap - written) as usize).min(buf.len());
        fill_pattern(&mut buf.as_mut()[..n], pattern, rng.as_mut());
        match file.write(&buf.as_ref()[..n]) {
            Ok(0) => break,
            Ok(k) => {
                written += k as u64;
                *bytes += k as u64;
            }
            Err(e) if is_no_space(&e) => {
                let _ = file.sync_all();
                if written == 0 {
                    let _ = fs::remove_file(path);
                    return Ok(false);
                }
                return Ok(true);
            }
            Err(e) => return Err(Error::from(e)),
        }
    }
    file.sync_all()?;
    // If we created a file but wrote nothing because the volume was already full.
    if written == 0 {
        let _ = fs::remove_file(path);
        return Ok(false);
    }
    Ok(true)
}

fn is_no_space(err: &std::io::Error) -> bool {
    // Unix ENOSPC = 28, Windows ERROR_DISK_FULL = 112, ERROR_HANDLE_DISK_FULL = 39.
    matches!(err.raw_os_error(), Some(28) | Some(112) | Some(39))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::method::Method;
    use crate::safety::SafetyFlags;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn cancel_mid_write_removes_filler_files() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("FRAME_WIPE_FORBID_REAL_DISKS", "1");
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("FRAME_WIPE_TEST_DIR", dir.path());
        TEST_FILE_CAP.store(512 * 1024 * 1024, Ordering::SeqCst);

        let mount = dir.path().to_path_buf();
        let name = mount.file_name().unwrap().to_string_lossy().into_owned();
        let flags = SafetyFlags {
            explicit_target: true,
            assume_yes: true,
            typed_confirm: Some(name),
            countdown_secs: 1,
            ..SafetyFlags::default()
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_w = cancel.clone();
        let mount_w = mount.clone();
        let handle = std::thread::spawn(move || {
            wipe_free_space(&mount_w, Method::zeros(), flags, &cancel_w, |_| {})
        });

        let fill_dir = mount.join(FILL_DIR);
        let first = fill_dir.join("fill-000000");
        let start = Instant::now();
        while !first.exists() {
            if start.elapsed() > Duration::from_secs(15) {
                cancel.store(true, Ordering::SeqCst);
                let _ = handle.join();
                TEST_FILE_CAP.store(0, Ordering::SeqCst);
                panic!("filler file was never created");
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        cancel.store(true, Ordering::SeqCst);
        let result = handle.join().expect("wipe thread panicked");
        TEST_FILE_CAP.store(0, Ordering::SeqCst);
        assert!(
            matches!(result, Err(Error::Cancelled)),
            "expected cancel, got {result:?}"
        );
        assert!(
            !first.exists(),
            "filler file left behind at {}",
            first.display()
        );
        assert!(
            !fill_dir.exists(),
            "filler directory left behind at {}",
            fill_dir.display()
        );
    }
}
