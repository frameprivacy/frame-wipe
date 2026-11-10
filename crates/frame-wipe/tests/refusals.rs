//! Every planner refusal path. These tests use synthetic snapshots only.
//! They run the same way on Linux, macOS, and Windows.

mod common;
use common::{device_snapshot, image_snapshot};
use frame_wipe::error::Guard;
use frame_wipe::planner::{plan, Scope};
use frame_wipe::safety::SafetyFlags;
use frame_wipe::Method;

fn flags_named_confirm(name: &str) -> SafetyFlags {
    SafetyFlags {
        explicit_target: true,
        typed_confirm: Some(name.into()),
        assume_yes: true,
        countdown_secs: 5,
        ..SafetyFlags::default()
    }
}

fn err_guard(target: frame_wipe::TargetSnapshot, scope: Scope, flags: SafetyFlags) -> Guard {
    match plan(&target, scope, Method::zeros(), None, &flags) {
        Err(e) => e
            .guard()
            .unwrap_or_else(|| panic!("expected a guard, got {e}")),
        Ok(_) => panic!("expected a refusal, got a plan"),
    }
}

#[test]
fn refuses_system_disk_without_flag() {
    let mut t = device_snapshot("sda", 1_000_000_000);
    t.is_system = true;
    let g = err_guard(t, Scope::WholeDevice, flags_named_confirm("sda"));
    assert_eq!(g, Guard::SystemDisk);
}

#[test]
fn allows_system_disk_with_flag_and_confirm() {
    let mut t = device_snapshot("sda", 1_000_000_000);
    t.is_system = true;
    let mut f = flags_named_confirm("sda");
    f.allow_system_disk = true;
    plan(&t, Scope::WholeDevice, Method::zeros(), None, &f).expect("should plan");
}

#[test]
fn refuses_app_disk_without_flag() {
    let mut t = device_snapshot("nvme0n1", 1_000_000_000);
    t.is_app_disk = true;
    let g = err_guard(t, Scope::WholeDevice, flags_named_confirm("nvme0n1"));
    assert_eq!(g, Guard::AppDisk);
}

#[test]
fn refuses_mounted_volume_except_free_space() {
    let mut t = device_snapshot("sdb1", 1_000_000_000);
    t.mounts = vec!["/data".into()];
    let g = err_guard(t.clone(), Scope::WholeDevice, flags_named_confirm("sdb1"));
    assert_eq!(g, Guard::Mounted);
    let mut f = flags_named_confirm("sdb1");
    // free-space is allowed on a mounted volume
    plan(&t, Scope::FreeSpace, Method::zeros(), None, &f).expect("free space should plan");
    f.allow_mounted = true;
    plan(&t, Scope::Partition, Method::zeros(), None, &f).expect("override");
}

#[test]
fn refuses_in_use() {
    let mut t = device_snapshot("sdc", 1_000_000_000);
    t.in_use = true;
    let g = err_guard(t, Scope::WholeDevice, flags_named_confirm("sdc"));
    assert_eq!(g, Guard::InUse);
}

#[test]
fn refuses_raid_member() {
    let mut t = device_snapshot("sdd", 1_000_000_000);
    t.is_raid_member = true;
    let g = err_guard(t, Scope::WholeDevice, flags_named_confirm("sdd"));
    assert_eq!(g, Guard::RaidMember);
}

#[test]
fn refuses_lvm_member() {
    let mut t = device_snapshot("sde", 1_000_000_000);
    t.is_lvm_member = true;
    let g = err_guard(t, Scope::WholeDevice, flags_named_confirm("sde"));
    assert_eq!(g, Guard::LvmMember);
}

#[test]
fn raid_flag_covers_lvm() {
    let mut t = device_snapshot("sdf", 1_000_000_000);
    t.is_lvm_member = true;
    let mut f = flags_named_confirm("sdf");
    f.allow_raid = true;
    plan(&t, Scope::WholeDevice, Method::zeros(), None, &f).expect("allow-raid covers LVM");
}

#[test]
fn refuses_missing_identity_on_block_device() {
    let mut t = device_snapshot("sdg", 1_000_000_000);
    t.identity.serial = None;
    t.identity.wwn = None;
    t.identity.file_id = None;
    let g = err_guard(t, Scope::WholeDevice, flags_named_confirm("sdg"));
    assert_eq!(g, Guard::MissingIdentity);
}

#[test]
fn refuses_missing_or_wrong_confirmation() {
    let t = device_snapshot("sdh", 1_000_000_000);
    let mut f = flags_named_confirm("sdh");
    f.typed_confirm = None;
    f.assume_yes = false;
    let g = err_guard(t.clone(), Scope::WholeDevice, f.clone());
    assert_eq!(g, Guard::ConfirmationMissing);
    f.typed_confirm = Some("sda".into());
    f.assume_yes = true;
    let g = err_guard(t, Scope::WholeDevice, f);
    assert_eq!(g, Guard::ConfirmationMismatch);
}

#[test]
fn refuses_unnamed_target() {
    let t = device_snapshot("sdi", 1_000_000_000);
    let mut f = flags_named_confirm("sdi");
    f.explicit_target = false;
    let g = err_guard(t, Scope::WholeDevice, f);
    assert_eq!(g, Guard::YesWithoutNamedTarget);
}

#[test]
fn refuses_symlink() {
    let mut t = image_snapshot("link", 4096);
    t.is_symlink = true;
    t.is_regular_file = false;
    let g = err_guard(t, Scope::Files, flags_named_confirm("link"));
    assert_eq!(g, Guard::Symlink);
}

#[test]
fn refuses_zero_countdown_on_physical_disk() {
    let t = device_snapshot("sdj", 1_000_000_000);
    let mut f = flags_named_confirm("sdj");
    f.countdown_secs = 0;
    let err = plan(&t, Scope::WholeDevice, Method::zeros(), None, &f).unwrap_err();
    assert!(err.to_string().contains("zero-second countdown"));
}

#[test]
fn allows_zero_countdown_on_image() {
    let t = image_snapshot("disk.img", 8 * 1024 * 1024);
    let mut f = flags_named_confirm("disk.img");
    f.countdown_secs = 0;
    plan(&t, Scope::WholeDevice, Method::zeros(), None, &f).expect("image + countdown 0");
}

#[test]
fn image_yes_without_confirm_is_allowed() {
    let t = image_snapshot("disk.img", 8 * 1024 * 1024);
    let f = SafetyFlags {
        explicit_target: true,
        assume_yes: true,
        typed_confirm: None,
        countdown_secs: 0,
        ..SafetyFlags::default()
    };
    plan(&t, Scope::WholeDevice, Method::zeros(), None, &f).expect("image --yes");
}

#[test]
fn firmware_sanitize_refused_on_image() {
    let t = image_snapshot("disk.img", 8 * 1024 * 1024);
    let err = plan(
        &t,
        Scope::FirmwareSanitize,
        Method::zeros(),
        None,
        &flags_named_confirm("disk.img"),
    )
    .unwrap_err();
    assert_eq!(err.guard(), Some(Guard::FirmwareNotSupported));
}

#[test]
fn default_single_pass_verifies() {
    let t = image_snapshot("disk.img", 8 * 1024 * 1024);
    let p = plan(
        &t,
        Scope::WholeDevice,
        Method::zeros(),
        None,
        &flags_named_confirm("disk.img"),
    )
    .unwrap();
    assert!(
        p.passes.iter().all(|x| x.verify) || p.passes.last().map(|x| x.verify).unwrap_or(false)
    );
    assert!(p.method.is_single_pass());
    assert!(p.passes[0].verify);
}

#[test]
fn confirmation_accepts_dev_prefix_normalization() {
    let t = device_snapshot("sdb", 1_000_000_000);
    let mut f = flags_named_confirm("/dev/sdb");
    plan(&t, Scope::WholeDevice, Method::zeros(), None, &f).expect("normalized name");
    f.typed_confirm = Some("sdb".into());
    plan(&t, Scope::WholeDevice, Method::zeros(), None, &f).expect("short name");
}
