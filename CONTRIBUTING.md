# Contributing

## Build and test

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Set `FRAME_WIPE_FORBID_REAL_DISKS=1` when you run tests. Tests must never write to a real physical disk. Use temp files, loop devices, ramdisks, or disk images only.

Integration helpers live in `scripts/`. On a developer machine, `scripts/ci-integration.sh` (Unix) or `scripts/ci-integration.ps1` (Windows) create a small image and run `framewipe` against it.

## Rules

- Do not add `unwrap` or `expect` on I/O paths in the library.
- New refusal reasons need a `Guard` variant, a planner test, and a line in SAFETY.md.
- Device identity (serial, size, path, file id) must still be re-checked immediately before the first write.
- Keep user-facing text in English and French.
- Do not claim NIST, DoD, or other certification.
- Commit messages should say what changed. Do not add tool trailers.

The CLI parse path must stay panic-free: add cases to the argument tests when you add flags.
