use frame_wipe::error::Guard;
use frame_wipe::files::wipe_tree;
use frame_wipe::safety::SafetyFlags;
use frame_wipe::Method;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[cfg(unix)]
#[test]
fn refuses_symlink() {
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
