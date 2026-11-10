# Safety

This document lists every guard in frame-wipe. Destructive work goes through the planner (`plan`) and the engine. The engine re-checks identity immediately before the first write. I/O paths return errors; they do not unwrap or panic on expected failure.

## Targets the tool will refuse

Unless an explicit flag is passed, the planner refuses:

| Guard | Meaning | Flag |
| --- | --- | --- |
| `SystemDisk` | Boot or system disk (the disk that holds `/`, `/boot`, or the Windows directory). | `--allow-system-disk` |
| `AppDisk` | The disk the running `framewipe` / `framewipe-gui` executable lives on. Also refuses to overwrite the executable itself. | `--allow-app-disk` |
| `Mounted` | The device or partition is mounted. Free-space mode and file mode are the exceptions. | `--allow-mounted` |
| `InUse` | Swap, exclusive-lock failure, or other holders. | `--allow-in-use` |
| `RaidMember` | A software RAID member (Linux md, Apple RAID, and similar). | `--allow-raid` |
| `LvmMember` | LVM or device-mapper member. | `--allow-raid` (same flag) |
| `MissingIdentity` | No serial, WWN, or file id for a block device. | `--allow-unidentified` |
| `ConfirmationMissing` | The device name was not typed. | `--confirm <name>` or the GUI field |
| `ConfirmationMismatch` | The typed text is not the device name. | Type the exact name (`sdb`, `disk3`, `PhysicalDrive1`, or the image file name). A `/dev/` or `\\.\` prefix is ignored. |
| `YesWithoutNamedTarget` | `--yes` without a target path or device name on the command line. | Name the target |
| `Symlink` / `FollowedLink` | The path is a symlink. File wipes never follow symlinks (`O_NOFOLLOW` / `FILE_FLAG_OPEN_REPARSE_POINT`, plus `lstat`). | Do not pass a symlink |
| `RestrictedTestTarget` | Tests and CI (`FRAME_WIPE_FORBID_REAL_DISKS=1`) may only write regular files or virtual devices whose backing file is under the test directory. | Not a user flag |
| `IdentityMismatch` | Serial, size, path, or file id changed after the plan, or a mount/holder appeared. | Start again after checking the device |
| `CountdownCancelled` | The operator cancelled during the pre-write countdown. No write is issued. | - |
| `FirmwareNotSupported` | Firmware sanitize is not available on this target, the platform cannot issue it, or `FRAME_WIPE_FORBID_REAL_DISKS` is set. | Use overwrite, or a vendor tool |

A zero-second countdown is allowed only for disk images and verified virtual devices, not for physical disks.

`--yes` does not skip `--confirm` on block devices. Image files may use `--yes` with the path named on the command line.

## Identity re-check

1. At plan time the tool records path, canonical path, display name, model, serial, WWN, size, and file id (inode or Windows file index).
2. After the countdown, and immediately before the first write, it recaptures that identity and compares it. A change (including a hot-plug that reuses the same path with a different serial or size) aborts.
3. It also re-reads mount, system, app-disk, RAID, and LVM state and aborts if that state changed.

## Countdown

Default is 5 seconds, cancellable (CLI: Ctrl-C; GUI: Cancel). The countdown runs after confirmation and before the identity re-check and the first write.

## Writes

- Direct I/O is used when the OS allows it (`O_DIRECT`, `F_NOCACHE`, `FILE_FLAG_NO_BUFFERING`). If that open fails, the tool falls back to buffered I/O and still `fsync`s.
- Short writes are retried once, then reported with the offset and the number of bytes actually written, and the run stops.
- Media errors (bad sectors) abort the run and report the offset, unless `--skip-bad-sectors` is set. In that case the range is recorded in the report and may still hold leftover data.
- Cancel finishes the current chunk, `fsync`s, writes a journal checkpoint, and returns a cancelled report.
- A journal file is flushed and `fsync`d as work proceeds, so a power loss can be detected on the next run. For a regular image file the journal is a sidecar (`.name.frame-wipe-journal`) next to the target. For a raw device it is stored under the application cache directory (`$XDG_CACHE_HOME/frame-wipe`, `~/Library/Caches/frame-wipe`, or `%LOCALAPPDATA%\frame-wipe`), because `/dev` and `\\.\` are not durable places for a sidecar. `--resume` continues only if the identity still matches.

## Verify

Single-pass methods verify by default. For a random pass, the same CSPRNG seed used to write is used to rebuild the expected pattern in memory; the seed is not written to the report.

## Firmware commands

On Linux, `framewipe sanitize` can issue NVMe Sanitize (crypto or block erase), NVMe Format NVM, or ATA SECURITY ERASE UNIT after the same planner checks as a wipe, including a typed confirmation of the device name. The tool first reads Identify data and refuses a frozen ATA security lock, a drive that already has a password, or a controller that does not report support.

Firmware commands are never sent when `FRAME_WIPE_FORBID_REAL_DISKS=1` (the default in tests and CI). macOS and Windows report capability only in this version and do not send a firmware command.

A wrong or interrupted firmware command can make a drive unusable or leave it locked. This tool is not NIST certified. Read the drive manual before using this path.

## Tests

Every refusal path above has a planner test that runs on Linux, macOS, and Windows (synthetic snapshots, no real disks). Integration tests overwrite only temp image files, and (in CI, when the runner allows it) loop devices, ramdisks, or VHD/disk images created for the test. `FRAME_WIPE_FORBID_REAL_DISKS=1` is set in CI.

If you find a way to bypass a guard, see [SECURITY.md](SECURITY.md).
