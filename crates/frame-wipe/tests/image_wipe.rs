//! Integration tests that only touch regular files in a temp directory.

use frame_wipe::engine::{wipe, WipeRequest, BETWEEN_PLAN_AND_RECHECK};
use frame_wipe::error::Guard;
use frame_wipe::identity::assert_same;
use frame_wipe::planner::Scope;
use frame_wipe::safety::SafetyFlags;
use frame_wipe::{device, Method, Outcome};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn setup() -> (MutexGuard<'static, ()>, tempfile::TempDir, PathBuf) {
    let guard = lock_env();
    std::env::set_var("FRAME_WIPE_FORBID_REAL_DISKS", "1");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().to_path_buf();
    std::env::set_var("FRAME_WIPE_TEST_DIR", &path);
    (guard, dir, path)
}

fn make_image(dir: &Path, name: &str, size: usize, fill: u8) -> PathBuf {
    let path = dir.join(name);
    let mut f = File::create(&path).unwrap();
    f.write_all(&vec![fill; size]).unwrap();
    f.sync_all().unwrap();
    path
}

fn flags_for(path: &Path) -> SafetyFlags {
    SafetyFlags {
        explicit_target: true,
        assume_yes: true,
        typed_confirm: Some(path.file_name().unwrap().to_string_lossy().into_owned()),
        countdown_secs: 0,
        ..SafetyFlags::default()
    }
}

#[test]
fn zeros_with_default_verify() {
    let (_env, tmp, dir) = setup();
    let path = make_image(&dir, "z.img", 2 * 1024 * 1024, 0xA5);
    let cancel = Arc::new(AtomicBool::new(false));
    let req = WipeRequest {
        path: path.clone(),
        scope: Scope::WholeDevice,
        method: Method::zeros(),
        chunk_size: Some(64 * 1024),
        flags: flags_for(&path),
    };
    let report = wipe(&req, &cancel, |_| {}).expect("wipe");
    assert_eq!(report.outcome, Outcome::Success);
    assert!(report.verified);
    let data = fs::read(&path).unwrap();
    assert!(data.iter().all(|b| *b == 0), "image should be all zeros");
    drop(tmp);
}

#[test]
fn random_with_verify() {
    let (_env, tmp, dir) = setup();
    let path = make_image(&dir, "r.img", 1024 * 1024, 0x11);
    let cancel = Arc::new(AtomicBool::new(false));
    let req = WipeRequest {
        path: path.clone(),
        scope: Scope::WholeDevice,
        method: Method::random(),
        chunk_size: Some(64 * 1024),
        flags: flags_for(&path),
    };
    let report = wipe(&req, &cancel, |_| {}).expect("wipe");
    assert_eq!(report.outcome, Outcome::Success);
    let data = fs::read(&path).unwrap();
    assert!(data.iter().any(|b| *b != 0x11), "pattern should change");
    drop(tmp);
}

#[test]
fn dry_run_does_not_write() {
    let (_env, tmp, dir) = setup();
    let path = make_image(&dir, "d.img", 64 * 1024, 0xAB);
    let mut flags = flags_for(&path);
    flags.dry_run = true;
    let req = WipeRequest {
        path: path.clone(),
        scope: Scope::WholeDevice,
        method: Method::zeros(),
        chunk_size: None,
        flags,
    };
    let report = wipe(&req, &Arc::new(AtomicBool::new(false)), |_| {}).unwrap();
    assert_eq!(report.outcome, Outcome::DryRun);
    let data = fs::read(&path).unwrap();
    assert!(data.iter().all(|b| *b == 0xAB));
    drop(tmp);
}

#[test]
fn identity_mismatch_aborts_before_write() {
    let (_env, tmp, dir) = setup();
    let path = make_image(&dir, "hot.img", 512 * 1024, 0xCD);
    // After plan+countdown, truncate the file so size changes.
    // The hook must use a process-wide leak of the path.
    let hook_path = path.clone();
    // Store path in env for the fn pointer hook.
    std::env::set_var("FRAME_WIPE_TEST_MUTATE", hook_path);
    fn mutate() {
        if let Ok(p) = std::env::var("FRAME_WIPE_TEST_MUTATE") {
            let _ = OpenTruncate::go(&p);
        }
    }
    struct OpenTruncate;
    impl OpenTruncate {
        fn go(p: &str) -> std::io::Result<()> {
            let f = std::fs::OpenOptions::new().write(true).open(p)?;
            f.set_len(256 * 1024)?;
            Ok(())
        }
    }
    {
        let mut hook = BETWEEN_PLAN_AND_RECHECK.lock().expect("hook lock");
        *hook = Some(mutate);
    }
    let req = WipeRequest {
        path: path.clone(),
        scope: Scope::WholeDevice,
        method: Method::zeros(),
        chunk_size: None,
        flags: flags_for(&path),
    };
    let err = wipe(&req, &Arc::new(AtomicBool::new(false)), |_| {}).unwrap_err();
    assert_eq!(err.guard(), Some(Guard::IdentityMismatch));
    // Original (now truncated) file should still contain 0xCD, not zeros.
    let data = fs::read(&path).unwrap();
    assert!(
        data.iter().all(|b| *b == 0xCD),
        "must not write after mismatch"
    );
    drop(tmp);
}

#[test]
fn recapture_detects_size_change() {
    let (_env, tmp, dir) = setup();
    let path = make_image(&dir, "id.img", 128 * 1024, 0x01);
    let first = device::inspect(&path).unwrap().identity;
    {
        let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.set_len(64 * 1024).unwrap();
    }
    let second = device::recapture(&path).unwrap();
    assert!(assert_same(&first, &second).is_err());
    drop(tmp);
}

#[test]
fn cancel_during_countdown() {
    use frame_wipe::confirm::countdown;
    use std::thread;
    use std::time::Duration;
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(120));
        c2.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let err = countdown(5, &cancel, |_| Ok(())).unwrap_err();
    assert_eq!(err.guard(), Some(Guard::CountdownCancelled));
}

#[test]
fn restricted_mode_refuses_raw_device_path() {
    let _env = lock_env();
    std::env::set_var("FRAME_WIPE_FORBID_REAL_DISKS", "1");
    let flags = SafetyFlags {
        explicit_target: true,
        assume_yes: true,
        typed_confirm: Some("sda".into()),
        countdown_secs: 0,
        ..SafetyFlags::default()
    };
    let req = WipeRequest {
        path: PathBuf::from("/dev/sda"),
        scope: Scope::WholeDevice,
        method: Method::zeros(),
        chunk_size: None,
        flags,
    };
    let err = wipe(&req, &Arc::new(AtomicBool::new(false)), |_| {}).unwrap_err();
    // May fail at inspect or at the restricted guard.
    let g = err.guard();
    assert!(
        g == Some(Guard::RestrictedTestTarget)
            || g.is_none()
            || g == Some(Guard::ConfirmationMissing),
        "got {err:?} guard={g:?}"
    );
}

#[test]
fn enumerate_does_not_panic() {
    let _ = device::enumerate(device::EnumerateOptions {
        include_virtual: true,
    });
}

#[test]
fn system_disk_is_detected_and_not_wiped() {
    let _env = lock_env();
    std::env::set_var("FRAME_WIPE_FORBID_REAL_DISKS", "1");
    let list = device::enumerate(device::EnumerateOptions {
        include_virtual: true,
    })
    .unwrap_or_default();
    let system: Vec<_> = list.iter().filter(|d| d.snapshot.is_system).collect();
    // On a normal machine there is a system disk. In unusual containers there may not be.
    for d in system {
        let flags = SafetyFlags {
            explicit_target: true,
            assume_yes: true,
            typed_confirm: Some(d.snapshot.identity.display_name.clone()),
            countdown_secs: 0,
            ..SafetyFlags::default()
        };
        let req = WipeRequest {
            path: d.snapshot.identity.path.clone(),
            scope: Scope::WholeDevice,
            method: Method::zeros(),
            chunk_size: None,
            flags: flags.clone(),
        };
        let err = wipe(&req, &Arc::new(AtomicBool::new(false)), |_| {}).unwrap_err();
        // Restricted mode or SystemDisk, never a successful write.
        assert!(
            err.guard() == Some(Guard::SystemDisk)
                || err.guard() == Some(Guard::RestrictedTestTarget)
                || err.guard() == Some(Guard::Mounted)
                || err.guard() == Some(Guard::AppDisk)
                || err.guard() == Some(Guard::InUse),
            "system wipe must be refused, got {err}"
        );
    }
}

#[test]
fn restricted_mode_refuses_windows_physical_drive_before_open() {
    let _env = lock_env();
    std::env::set_var("FRAME_WIPE_FORBID_REAL_DISKS", "1");
    let flags = SafetyFlags {
        explicit_target: true,
        assume_yes: true,
        typed_confirm: Some("PhysicalDrive0".into()),
        countdown_secs: 0,
        ..SafetyFlags::default()
    };
    let req = WipeRequest {
        path: std::path::PathBuf::from(r"\\.\PhysicalDrive0"),
        scope: Scope::WholeDevice,
        method: Method::zeros(),
        chunk_size: None,
        flags,
    };
    let err = wipe(&req, &Arc::new(AtomicBool::new(false)), |_| {}).unwrap_err();
    assert_eq!(
        err.guard(),
        Some(Guard::RestrictedTestTarget),
        "must refuse before opening a raw device, got {err}"
    );
}

#[test]
fn resume_without_flag_refuses_and_with_flag_skips_already_written_bytes() {
    use frame_wipe::journal;
    use std::sync::atomic::Ordering;
    use std::thread;
    use std::time::{Duration, Instant};

    let (_env, tmp, dir) = setup();
    let path = make_image(&dir, "resume.img", 4 * 1024 * 1024, 0xA5);
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_w = cancel.clone();
    let path_w = path.clone();
    let flags = flags_for(&path);
    let handle = thread::spawn(move || {
        let req = WipeRequest {
            path: path_w,
            scope: Scope::WholeDevice,
            method: Method::Single {
                pattern: frame_wipe::method::Pattern::Zeros,
                verify: false,
            },
            chunk_size: Some(64 * 1024),
            flags,
        };
        wipe(&req, &cancel_w, |p| {
            if p.phase == frame_wipe::progress::Phase::Writing && p.bytes_done >= 512 * 1024 {
                cancel_w.store(true, Ordering::SeqCst);
            }
        })
    });
    let started = Instant::now();
    let result = loop {
        if handle.is_finished() {
            break handle.join().expect("wipe thread panicked");
        }
        if started.elapsed() > Duration::from_secs(30) {
            cancel.store(true, Ordering::SeqCst);
            let result = handle.join().expect("wipe thread panicked");
            panic!("wipe did not cancel in time: {result:?}");
        }
        thread::sleep(Duration::from_millis(5));
    };
    let report = result.expect("cancelled wipe should return a report");
    assert_eq!(report.outcome, Outcome::Cancelled);

    let journal_path = journal::journal_path_for(&path);
    let journal = journal::read(&journal_path)
        .unwrap()
        .expect("incomplete wipe must leave a journal");
    let resume_at = journal.offset;
    assert!(
        resume_at > 0 && resume_at < 4 * 1024 * 1024,
        "journal offset should be mid-wipe, got {resume_at}"
    );

    // Mark the already-written prefix so a restart from offset 0 would overwrite the marker.
    {
        let mut f = fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.write_all(&vec![0x11u8; resume_at as usize]).unwrap();
        f.sync_all().unwrap();
    }

    let mut flags = flags_for(&path);
    let err = wipe(
        &WipeRequest {
            path: path.clone(),
            scope: Scope::WholeDevice,
            method: Method::Single {
                pattern: frame_wipe::method::Pattern::Zeros,
                verify: false,
            },
            chunk_size: Some(64 * 1024),
            flags: flags.clone(),
        },
        &Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap_err();
    assert!(
        matches!(err, frame_wipe::Error::Journal(_)),
        "a second start without --resume must refuse, got {err}"
    );

    flags.resume = true;
    let report = wipe(
        &WipeRequest {
            path: path.clone(),
            scope: Scope::WholeDevice,
            method: Method::Single {
                pattern: frame_wipe::method::Pattern::Zeros,
                verify: false,
            },
            chunk_size: Some(64 * 1024),
            flags,
        },
        &Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .expect("resume");
    assert_eq!(report.outcome, Outcome::Success);
    let data = fs::read(&path).unwrap();
    assert!(
        data[..resume_at as usize].iter().all(|b| *b == 0x11),
        "resume must not rewrite bytes before the journal offset"
    );
    assert!(
        data[resume_at as usize..].iter().all(|b| *b == 0),
        "resume must finish zeroing from the journal offset"
    );
    assert!(
        !journal_path.exists(),
        "completed resume should remove the journal"
    );
    drop(tmp);
}

#[test]
fn cancel_during_verify_returns_cancelled_outcome() {
    use std::sync::atomic::Ordering;
    use std::thread;
    use std::time::{Duration, Instant};

    let (_env, tmp, dir) = setup();
    let path = make_image(&dir, "verify-cancel.img", 4 * 1024 * 1024, 0xA5);
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_w = cancel.clone();
    let path_w = path.clone();
    let flags = flags_for(&path);
    let handle = thread::spawn(move || {
        let req = WipeRequest {
            path: path_w,
            scope: Scope::WholeDevice,
            method: Method::zeros(),
            chunk_size: Some(64 * 1024),
            flags,
        };
        wipe(&req, &cancel_w, |p| {
            if p.phase == frame_wipe::progress::Phase::Verifying && p.bytes_done > 0 {
                cancel_w.store(true, Ordering::SeqCst);
            }
        })
    });
    let started = Instant::now();
    let result = loop {
        if handle.is_finished() {
            break handle.join().expect("wipe thread panicked");
        }
        if started.elapsed() > Duration::from_secs(30) {
            cancel.store(true, Ordering::SeqCst);
            let result = handle.join().expect("wipe thread panicked");
            panic!("wipe did not finish in time: {result:?}");
        }
        thread::sleep(Duration::from_millis(5));
    };
    let report = result.expect("cancel during verify should be a cancelled report, not an error");
    assert_eq!(report.outcome, Outcome::Cancelled);
    drop(tmp);
}

#[test]
fn system_volume_directory_is_flagged_and_free_space_is_refused() {
    use frame_wipe::planner::plan;

    let _env = lock_env();
    std::env::set_var("FRAME_WIPE_FORBID_REAL_DISKS", "1");
    let root = system_root_dir();
    let snap = device::inspect(root).expect("inspect system root");
    assert!(
        snap.is_directory,
        "the system root should inspect as a directory, got {snap:?}"
    );
    assert!(
        snap.is_system,
        "the directory that holds the OS must be flagged as the system volume so free-space mode requires --allow-system-disk"
    );
    let flags = SafetyFlags {
        explicit_target: true,
        assume_yes: true,
        typed_confirm: Some(snap.identity.display_name.clone()),
        countdown_secs: 0,
        ..SafetyFlags::default()
    };
    let err = plan(&snap, Scope::FreeSpace, Method::zeros(), None, &flags).unwrap_err();
    assert_eq!(err.guard(), Some(Guard::SystemDisk), "got {err}");
}

fn system_root_dir() -> &'static Path {
    #[cfg(windows)]
    {
        Path::new(r"C:\")
    }
    #[cfg(not(windows))]
    {
        Path::new("/")
    }
}
