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
