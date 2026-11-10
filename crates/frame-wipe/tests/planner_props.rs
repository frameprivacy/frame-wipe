//! Property tests on the planner.

use frame_wipe::error::Guard;
use frame_wipe::identity::DeviceIdentity;
use frame_wipe::planner::{normalize_chunk_size, plan, MediaKind, Scope, TargetSnapshot};
use frame_wipe::safety::SafetyFlags;
use frame_wipe::Method;
use proptest::prelude::*;
use std::path::PathBuf;

fn snapshot(
    name: String,
    size: u64,
    is_system: bool,
    is_app: bool,
    mounted: bool,
    in_use: bool,
    raid: bool,
    lvm: bool,
    identified: bool,
    regular: bool,
    symlink: bool,
) -> TargetSnapshot {
    TargetSnapshot {
        identity: DeviceIdentity {
            path: PathBuf::from(if regular {
                format!("/tmp/{name}")
            } else {
                format!("/dev/{name}")
            }),
            canonical_path: PathBuf::from(format!("/dev/{name}")),
            display_name: name.clone(),
            model: Some("PROP".into()),
            serial: if identified { Some("S".into()) } else { None },
            wwn: None,
            size,
            file_id: if identified || regular {
                Some((1, 1))
            } else {
                None
            },
            media: "hdd".into(),
        },
        kind: if regular {
            MediaKind::Image
        } else {
            MediaKind::Hdd
        },
        is_system,
        is_app_disk: is_app,
        mounts: if mounted { vec!["/mnt".into()] } else { vec![] },
        in_use,
        is_raid_member: raid,
        is_lvm_member: lvm,
        backing_file: if regular {
            Some(PathBuf::from(format!("/tmp/{name}")))
        } else {
            None
        },
        is_symlink: symlink,
        is_regular_file: regular && !symlink,
        is_directory: false,
        is_block_device: !regular,
        logical_block_size: 512,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn chunk_size_is_aligned_and_bounded(req in proptest::option::of(1u64..256 * 1024 * 1024), block in prop_oneof![Just(512u64), Just(4096u64), Just(8192u64)]) {
        let c = normalize_chunk_size(req, block);
        prop_assert!(c >= block);
        prop_assert!(c <= 128 * 1024 * 1024);
        prop_assert_eq!(c % block, 0);
    }

    #[test]
    fn dangerous_targets_are_refused(
        is_system in any::<bool>(),
        is_app in any::<bool>(),
        mounted in any::<bool>(),
        in_use in any::<bool>(),
        raid in any::<bool>(),
        lvm in any::<bool>(),
        identified in any::<bool>(),
        size in 4096u64..10_000_000_000,
    ) {
        let t = snapshot("sdb".into(), size, is_system, is_app, mounted, in_use, raid, lvm, identified, false, false);
        let flags = SafetyFlags {
            explicit_target: true,
            typed_confirm: Some("sdb".into()),
            assume_yes: true,
            countdown_secs: 5,
            ..SafetyFlags::default()
        };
        let result = plan(&t, Scope::WholeDevice, Method::zeros(), None, &flags);
        if is_system {
            prop_assert_eq!(result.unwrap_err().guard(), Some(Guard::SystemDisk));
        } else if is_app {
            prop_assert_eq!(result.unwrap_err().guard(), Some(Guard::AppDisk));
        } else if mounted {
            prop_assert_eq!(result.unwrap_err().guard(), Some(Guard::Mounted));
        } else if in_use {
            prop_assert_eq!(result.unwrap_err().guard(), Some(Guard::InUse));
        } else if raid {
            prop_assert_eq!(result.unwrap_err().guard(), Some(Guard::RaidMember));
        } else if lvm {
            prop_assert_eq!(result.unwrap_err().guard(), Some(Guard::LvmMember));
        } else if !identified {
            prop_assert_eq!(result.unwrap_err().guard(), Some(Guard::MissingIdentity));
        } else {
            prop_assert!(result.is_ok());
        }
    }

    #[test]
    fn wrong_confirmation_never_plans(typed in "[A-Za-z0-9._-]{1,32}") {
        prop_assume!(typed != "sdb" && typed != "/dev/sdb");
        let t = snapshot("sdb".into(), 1_000_000, false, false, false, false, false, false, true, false, false);
        let flags = SafetyFlags {
            explicit_target: true,
            typed_confirm: Some(typed),
            assume_yes: true,
            countdown_secs: 5,
            ..SafetyFlags::default()
        };
        let err = plan(&t, Scope::WholeDevice, Method::zeros(), None, &flags).unwrap_err();
        prop_assert_eq!(err.guard(), Some(Guard::ConfirmationMismatch));
    }

    #[test]
    fn dry_run_plan_has_dry_run_set(size in 4096u64..1_000_000u64) {
        let t = snapshot("disk.img".into(), size, false, false, false, false, false, false, true, true, false);
        let flags = SafetyFlags {
            explicit_target: true,
            assume_yes: true,
            dry_run: true,
            countdown_secs: 0,
            ..SafetyFlags::default()
        };
        let p = plan(&t, Scope::WholeDevice, Method::zeros(), None, &flags).unwrap();
        prop_assert!(p.dry_run);
        prop_assert_eq!(p.size, size);
    }
}
