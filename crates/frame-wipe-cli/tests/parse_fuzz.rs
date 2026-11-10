use framewipe_cli::parse_from;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn random_argv_does_not_panic(args in prop::collection::vec(".*", 0..12)) {
        let mut argv = vec!["framewipe".to_string()];
        argv.extend(args);
        let _ = parse_from(&argv);
    }

    #[test]
    fn structured_wipe_args_do_not_panic(
        method in prop::sample::select(vec!["zeros", "random", "ones", "multi", "bogus", ""]),
        yes in any::<bool>(),
        confirm in prop::option::of("[A-Za-z0-9._-]{0,40}"),
        countdown in 0u32..20,
        extra in prop::collection::vec(prop::sample::select(vec![
            "--json", "--dry-run", "--no-verify", "--verify",
            "--allow-system-disk", "--allow-raid", "--skip-bad-sectors",
        ]), 0..6),
    ) {
        let mut argv = vec![
            "framewipe".into(),
            "wipe".into(),
            "/tmp/disk.img".into(),
            "--method".into(),
            method.to_string(),
            "--countdown".into(),
            countdown.to_string(),
        ];
        if yes {
            argv.push("--yes".into());
        }
        if let Some(c) = confirm {
            argv.push("--confirm".into());
            argv.push(c);
        }
        argv.extend(extra.into_iter().map(|s| s.to_string()));
        let _ = parse_from(&argv);
    }
}
