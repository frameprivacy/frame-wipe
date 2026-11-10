# frame-wipe

Overwrite disks, partitions, free space, or files on Linux, macOS (Apple Silicon), and Windows.

This is a Frame Labs Inc. project. The command-line binary is `framewipe`. The desktop binary is `framewipe-gui`.

## What it does

- Overwrite a whole disk, a partition, or a disk image file with zeros, a CSPRNG random pattern (ChaCha20), or a three-pass zeros/random/zeros sequence.
- Overwrite free space on a mounted volume, then delete the filler files.
- Overwrite regular files or a directory tree. Symlinks are never followed.
- Report whether an NVMe or ATA firmware sanitize command appears to be available. This version does not send a sanitize or ATA Secure Erase command.
- Write a JSON and plain-text completion report. The report is a record of what the tool did, not a certification.

Single-pass methods run a verify read by default. Use `--no-verify` to turn that off.

## What it does not do

- It is not NIST, DoD, or Common Criteria certified.
- Overwrite on an SSD or NVMe device cannot reliably reach overprovisioned or remapped blocks. That is closer to NIST SP 800-88 *Clear* than *Purge*. Prefer a firmware sanitize from the vendor or `nvme-cli` when you need a purge, and read the drive manual.
- It will not wipe the boot/system disk, the disk the program is running from, a mounted or in-use volume, or a RAID/LVM member unless you pass an explicit flag and type the device name.

See [SAFETY.md](SAFETY.md) for every guard.

## Requirements

- Rust 1.85 or later to build from source (CI uses current stable).
- Administrator / root rights to open raw disks (`/dev/sdX` or `/dev/nvmeXnY` on Linux, `/dev/rdiskN` on macOS, `\\.\PhysicalDriveN` on Windows).
- macOS builds are Apple Silicon only. Intel Mac is not supported.

## Build

```
cargo build --release -p frame-wipe-cli
cargo build --release -p frame-wipe-gui
```

The binaries are `target/release/framewipe` and `target/release/framewipe-gui`.

## Command line

```
framewipe list
framewipe list --all
framewipe wipe /path/to/disk.img --yes --confirm disk.img --countdown 0
framewipe wipe /dev/sdb --method random --confirm sdb
framewipe files ./secret --yes --confirm secret
framewipe free-space /mnt/data --confirm data
framewipe sanitize /dev/nvme1n1 --confirm nvme1n1
```

Useful flags:

- `--method zeros|random|ones|multi`
- `--verify` / `--no-verify`
- `--chunk-size 8M`
- `--dry-run`
- `--yes` only works when the target is named on the command line. Block devices still require `--confirm <device-name>`.
- `--confirm <name>` is the typed confirmation of the device name (`sdb`, `disk3`, `PhysicalDrive1`, or the image file name).
- `--allow-system-disk`, `--allow-app-disk`, `--allow-mounted`, `--allow-raid` (also covers LVM), `--allow-in-use`, `--allow-unidentified`
- `--skip-bad-sectors` records media errors and continues. Those ranges may still hold data.
- `--resume` continues from a journal left by a previous interrupted run, only if the device identity still matches.
- `--countdown 5` (default). `0` is allowed only for disk images and verified virtual devices.
- `--report path/prefix` writes `.json` and `.txt`.
- `--json` prints machine-readable output.
- `--lang en|fr`

A 5-second cancellable countdown runs before the first write. Ctrl-C cancels after the current chunk and leaves a journal.

## Desktop UI

`framewipe-gui` lists devices, asks you to type the device name, then runs the same countdown and the same library as the CLI. English and French are available.

`framewipe-gui --smoke-test` opens the window, draws a few frames, and exits. Used by CI.

The GUI is a native Rust interface (egui + eframe, glow renderer). It is not Electron or a system webview. A Linux x64 release build of this tree (LTO, stripped) is about 1.07 MiB for `framewipe` and about 4.84 MiB for `framewipe-gui`. CI artifacts have the sizes for the other OS and architecture combinations. macOS artifacts are Apple Silicon only.

On one Linux VM, overwriting a 512 MiB image file: `dd if=/dev/zero bs=8M conv=fsync` was about 583 MiB/s; `framewipe --method zeros --no-verify --chunk-size 8M` was about 940 MiB/s. That is a single machine, not a claim about every disk.

## Safety tests

Tests write only to temporary regular files, loop devices, ramdisks, or disk images (VHD / `.dmg` / image files) created by the test harness. They never overwrite a real physical disk. CI sets `FRAME_WIPE_FORBID_REAL_DISKS=1`.

## License

MIT. Copyright (c) 2026 Frame Labs Inc. See [LICENSE](LICENSE).
