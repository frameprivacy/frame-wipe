use frame_wipe::error::Guard;
use frame_wipe::files::wipe_tree;
use frame_wipe::safety::SafetyFlags;
use frame_wipe::Method;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(unix)]
#[test]
fn refuses_symlink() {
    let _env = lock_env();
    std::env::set_var("FRAME_WIPE_FORBID_REAL_DISKS", "1");
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("FRAME_WIPE_TEST_DIR", dir.path());
    let target = dir.path().join("real.txt");
    fs::write(&target, b"hello").unwrap();
    let link = dir.path().join("link.txt");
    symlink(&target, &link).unwrap();
    let flags = SafetyFlags {
        explicit_target: true,
        assume_yes: true,
        typed_confirm: Some("link.txt".into()),
        countdown_secs: 0,
        ..SafetyFlags::default()
    };
    let err = wipe_tree(
        &link,
        Method::zeros(),
        None,
        flags,
        &Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap_err();
    assert_eq!(err.guard(), Some(Guard::Symlink));
    assert_eq!(fs::read(&target).unwrap(), b"hello");
}

#[test]
fn wipes_regular_file() {
    let _env = lock_env();
    std::env::set_var("FRAME_WIPE_FORBID_REAL_DISKS", "1");
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("FRAME_WIPE_TEST_DIR", dir.path());
    let path = dir.path().join("doc.txt");
    fs::write(&path, b"secret-data-here!!").unwrap();
    let flags = SafetyFlags {
        explicit_target: true,
        assume_yes: true,
        typed_confirm: Some("doc.txt".into()),
        countdown_secs: 0,
        ..SafetyFlags::default()
    };
    let reports = wipe_tree(
        &path,
        Method::zeros(),
        None,
        flags,
        &Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    assert_eq!(reports.len(), 1);
    let data = fs::read(&path).unwrap();
    assert_eq!(data.len(), 18);
    assert!(data.iter().all(|b| *b == 0));
}

#[cfg(windows)]
#[test]
fn refuses_symlink_windows() {
    let _env = lock_env();
    std::env::set_var("FRAME_WIPE_FORBID_REAL_DISKS", "1");
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("FRAME_WIPE_TEST_DIR", dir.path());
    let target = dir.path().join("real.txt");
    fs::write(&target, b"hello").unwrap();
    let link = dir.path().join("link.txt");
    if std::os::windows::fs::symlink_file(&target, &link).is_err() {
        // Creating symlinks can require a privilege on Windows; skip in that case.
        return;
    }
    let flags = SafetyFlags {
        explicit_target: true,
        assume_yes: true,
        typed_confirm: Some("link.txt".into()),
        countdown_secs: 0,
        ..SafetyFlags::default()
    };
    let err = wipe_tree(
        &link,
        Method::zeros(),
        None,
        flags,
        &Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap_err();
    assert_eq!(err.guard(), Some(Guard::Symlink));
    assert_eq!(fs::read(&target).unwrap(), b"hello");
}

#[test]
fn refuses_read_only_file_and_leaves_content() {
    let _env = lock_env();
    std::env::set_var("FRAME_WIPE_FORBID_REAL_DISKS", "1");
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("FRAME_WIPE_TEST_DIR", dir.path());
    let path = dir.path().join("readonly.txt");
    let original = b"do-not-touch-me!!";
    fs::write(&path, original).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&path, perms).unwrap();
    let flags = SafetyFlags {
        explicit_target: true,
        assume_yes: true,
        typed_confirm: Some("readonly.txt".into()),
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
    let msg = err.to_string().to_ascii_lowercase();
    assert!(
        msg.contains("permission denied") || msg.contains("not writable") || msg.contains("access"),
        "unexpected error: {err}"
    );
    assert!(
        !msg.contains("needs elevation") && !msg.contains("elevation"),
        "{err}"
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_readonly(false);
    fs::set_permissions(&path, perms).unwrap();
}

#[test]
fn wipes_unicode_and_long_file_names() {
    let _env = lock_env();
    std::env::set_var("FRAME_WIPE_FORBID_REAL_DISKS", "1");
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("FRAME_WIPE_TEST_DIR", dir.path());
    let unicode = dir.path().join("café-文档-äé.txt");
    fs::write(&unicode, b"secret-unicode!!").unwrap();
    let long_name = format!("wiped-{}-end.bin", "あ".repeat(80));
    assert!(long_name.chars().count() > 80);
    let long_path = dir.path().join(&long_name);
    fs::write(&long_path, b"secret-long-name!").unwrap();

    for (path, confirm, original) in [
        (
            unicode.as_path(),
            "café-文档-äé.txt",
            &b"secret-unicode!!"[..],
        ),
        (
            long_path.as_path(),
            long_name.as_str(),
            &b"secret-long-name!"[..],
        ),
    ] {
        let flags = SafetyFlags {
            explicit_target: true,
            assume_yes: true,
            typed_confirm: Some(confirm.to_string()),
            countdown_secs: 0,
            ..SafetyFlags::default()
        };
        let reports = wipe_tree(
            path,
            Method::zeros(),
            None,
            flags,
            &Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        assert_eq!(reports.len(), 1);
        let data = fs::read(path).unwrap();
        assert_eq!(data.len(), original.len());
        assert!(data.iter().all(|b| *b == 0), "wipe did not zero {confirm}");
    }
}
