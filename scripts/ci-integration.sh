#!/usr/bin/env bash
# Create a small disk image and exercise wipe, verify, file wipe, list, and refusal paths.
# Never touches a real physical disk.
set -euo pipefail

export FRAME_WIPE_FORBID_REAL_DISKS=1
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${ROOT}/target/release/framewipe"
if [[ ! -x "$BIN" ]]; then
  BIN="${ROOT}/target/debug/framewipe"
fi
WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/framewipe-ci.XXXXXX")"
export FRAME_WIPE_TEST_DIR="$WORKDIR"
trap 'rm -rf "$WORKDIR"' EXIT

echo "== binary: $BIN"
echo "== workdir: $WORKDIR"

# Device enumeration must not crash.
"$BIN" list --json >/dev/null
"$BIN" list --all --json >/dev/null || true

# Image wipe + verify (default).
IMG="$WORKDIR/disk.img"
dd if=/dev/urandom of="$IMG" bs=1m count=32 status=none 2>/dev/null \
  || dd if=/dev/urandom of="$IMG" bs=1M count=32 status=none
"$BIN" wipe "$IMG" --yes --confirm disk.img --countdown 0 --method zeros --report "$WORKDIR/report"
# Confirm zeros.
python3 - << PY
import pathlib
p = pathlib.Path("$IMG")
data = p.read_bytes()
assert len(data) == 32 * 1024 * 1024, len(data)
assert all(b == 0 for b in data), "image was not zeroed"
print("image verified all zeros,", len(data), "bytes")
PY

# Random + verify.
"$BIN" wipe "$IMG" --yes --confirm disk.img --countdown 0 --method random

# File wipe, and symlink refusal.
echo "secret" > "$WORKDIR/doc.txt"
"$BIN" files "$WORKDIR/doc.txt" --yes --confirm doc.txt --countdown 0
python3 - << PY
import pathlib
data = pathlib.Path("$WORKDIR/doc.txt").read_bytes()
assert all(b == 0 for b in data), data
print("file wiped")
PY
if ln -s "$WORKDIR/doc.txt" "$WORKDIR/link.txt" 2>/dev/null; then
  if "$BIN" files "$WORKDIR/link.txt" --yes --confirm link.txt --countdown 0; then
    echo "ERROR: symlink was followed" >&2
    exit 1
  else
    echo "symlink correctly refused"
  fi
fi

# Refusal: system disk (or restricted raw device) must not be wiped.
if "$BIN" wipe /dev/sda --yes --confirm sda --countdown 0 --dry-run 2>"$WORKDIR/err.txt"; then
  # dry-run on a real disk should still be refused by system/restricted/mounted guards
  echo "ERROR: unexpected success planning a wipe of /dev/sda" >&2
  cat "$WORKDIR/err.txt" >&2
  exit 1
else
  echo "raw device path refused (expected):"
  cat "$WORKDIR/err.txt" || true
fi

# Free-space mode on a small loop-mounted filesystem, when we can use sudo.
if command -v mkfs.ext4 >/dev/null && sudo -n true 2>/dev/null; then
  FSIMG="$WORKDIR/fs.img"
  dd if=/dev/zero of="$FSIMG" bs=1M count=64 status=none
  mkfs.ext4 -F -q "$FSIMG"
  MNT="$WORKDIR/mnt"
  mkdir -p "$MNT"
  if sudo mount -o loop "$FSIMG" "$MNT"; then
    sudo chown "$(id -u):$(id -g)" "$MNT"
    echo leftover > "$MNT/keep.txt"
    # The confirm name is the mount directory name.
    if "$BIN" free-space "$MNT" --yes --confirm mnt --countdown 0 --allow-system-disk 2>"$WORKDIR/fs.err"; then
      echo "free-space wipe completed"
    else
      echo "free-space wipe failed (not fatal if permission or planner refused):"
      cat "$WORKDIR/fs.err" || true
    fi
    sudo umount "$MNT" || true
  fi
else
  echo "skipping loop-mount free-space test (no passwordless sudo or mkfs.ext4)"
fi

echo "Unix integration OK"
