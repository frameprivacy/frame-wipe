#!/usr/bin/env bash
# Compare framewipe zeros throughput with dd on a regular image file.
set -euo pipefail
export FRAME_WIPE_FORBID_REAL_DISKS=1
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${ROOT}/target/release/framewipe"
WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/framewipe-bench.XXXXXX")"
export FRAME_WIPE_TEST_DIR="$WORKDIR"
trap 'rm -rf "$WORKDIR"' EXIT
IMG="$WORKDIR/bench.img"
SIZE_MB="${BENCH_SIZE_MB:-512}"
echo "Preparing ${SIZE_MB}MiB image at $IMG"
dd if=/dev/zero of="$IMG" bs=1M count="$SIZE_MB" status=none

echo "== dd conv=fsync bs=8M"
START=$(date +%s.%N)
dd if=/dev/zero of="$IMG" bs=8M count=$((SIZE_MB / 8)) conv=fsync status=none
END=$(date +%s.%N)
python3 - << PY
start, end, size = float("$START"), float("$END"), ${SIZE_MB} * 1024 * 1024
dt = end - start
print(f"dd: {size/dt/1024/1024:.1f} MiB/s  ({dt:.3f}s)")
PY

echo "== framewipe --method zeros --no-verify --countdown 0"
START=$(date +%s.%N)
"$BIN" wipe "$IMG" --yes --confirm bench.img --countdown 0 --method zeros --no-verify --chunk-size 8M
END=$(date +%s.%N)
python3 - << PY
start, end, size = float("$START"), float("$END"), ${SIZE_MB} * 1024 * 1024
dt = end - start
print(f"framewipe: {size/dt/1024/1024:.1f} MiB/s  ({dt:.3f}s)")
PY
