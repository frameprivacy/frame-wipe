#!/usr/bin/env bash
# Capture every GUI demo state at 1024x700, 1280x800, and 1920x1080 under a
# private Xvfb, plus a 2x HiDPI set at 1280x800 logical pixels.
set -euo pipefail

BIN="${BIN:-$(cd "$(dirname "$0")/.." && pwd)/target/release/framewipe-gui}"
OUT="${OUT:-/workspace/frame/shots/zero-app}"
DISPLAY_NUM="${DISPLAY_NUM:-43}"
export DISPLAY=":${DISPLAY_NUM}"
export LIBGL_ALWAYS_SOFTWARE="${LIBGL_ALWAYS_SOFTWARE:-1}"

mkdir -p "$OUT"

STATES=(empty error list selected confirm countdown progress done failed)
SIZES=("1024x700" "1280x800" "1920x1080")

XPID=""
GPID=""
cleanup() {
  if [[ -n "${GPID}" ]]; then kill "${GPID}" 2>/dev/null || true; wait "${GPID}" 2>/dev/null || true; fi
  if [[ -n "${XPID}" ]]; then kill "${XPID}" 2>/dev/null || true; wait "${XPID}" 2>/dev/null || true; fi
}
trap cleanup EXIT

start_x() {
  local w="$1" h="$2"
  if [[ -n "${XPID}" ]]; then
    kill "${XPID}" 2>/dev/null || true
    wait "${XPID}" 2>/dev/null || true
    XPID=""
  fi
  # Drop any stale lock from a previous crash.
  rm -f "/tmp/.X${DISPLAY_NUM}-lock" "/tmp/.X11-unix/X${DISPLAY_NUM}"
  Xvfb ":${DISPLAY_NUM}" -screen 0 "${w}x${h}x24" -ac +extension GLX +render -noreset -nocursor >/tmp/fw-xvfb.log 2>&1 &
  XPID=$!
  local i
  for i in $(seq 1 50); do
    if xdpyinfo -display ":${DISPLAY_NUM}" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  echo "Xvfb failed to start on :${DISPLAY_NUM}" >&2
  cat /tmp/fw-xvfb.log >&2 || true
  return 1
}

capture_one() {
  local state="$1" w="$2" h="$3" scale="$4" dest="$5"
  local ready="/tmp/fw-gui-ready-$$"
  rm -f "${ready}"
  "${BIN}" --demo --demo-state "${state}" --width "${w}" --height "${h}" --scale "${scale}" --undecorated --ready-file "${ready}" >/tmp/fw-gui.log 2>&1 &
  GPID=$!
  local i
  for i in $(seq 1 100); do
    if [[ -f "${ready}" ]]; then
      break
    fi
    if ! kill -0 "${GPID}" 2>/dev/null; then
      echo "GUI exited before ready (${state} ${w}x${h} scale=${scale})" >&2
      cat /tmp/fw-gui.log >&2 || true
      GPID=""
      return 1
    fi
    sleep 0.1
  done
  if [[ ! -f "${ready}" ]]; then
    echo "timed out waiting for ready file (${state} ${w}x${h})" >&2
    cat /tmp/fw-gui.log >&2 || true
    kill "${GPID}" 2>/dev/null || true
    wait "${GPID}" 2>/dev/null || true
    GPID=""
    return 1
  fi
  echo "ready (${state} ${w}x${h} scale=${scale}):"
  cat "${ready}"
  # For scale S, the window inner size is w x h physical pixels and the
  # logical size should be (w/S) x (h/S).
  logical_line=$(grep -E "^logical=" "${ready}" || true)
  want_logical="$(python3 -c "print(int(round(${w} / ${scale})))")x$(python3 -c "print(int(round(${h} / ${scale})))")"
  if [[ "${logical_line}" != "logical=${want_logical}" ]]; then
    echo "logical size mismatch: ${logical_line} (expected logical=${want_logical})" >&2
    kill "${GPID}" 2>/dev/null || true
    wait "${GPID}" 2>/dev/null || true
    GPID=""
    return 1
  fi
  # Let eframe present at least one fully composed frame.
  sleep 0.5
  # Keep the pointer off the framebuffer so it is not in screenshots.
  xdotool mousemove -- 10000 10000 >/dev/null 2>&1 || true
  ffmpeg -y -hide_banner -loglevel error \
    -f x11grab -video_size "${w}x${h}" -i ":${DISPLAY_NUM}" \
    -frames:v 1 "${dest}"
  kill "${GPID}" 2>/dev/null || true
  wait "${GPID}" 2>/dev/null || true
  GPID=""
  echo "wrote ${dest} ($(wc -c < "${dest}") bytes)"
}

if [[ ! -x "${BIN}" ]]; then
  echo "missing binary: ${BIN}" >&2
  exit 1
fi

for size in "${SIZES[@]}"; do
  w="${size%x*}"
  h="${size#*x}"
  start_x "${w}" "${h}"
  for state in "${STATES[@]}"; do
    capture_one "${state}" "${w}" "${h}" 1 "${OUT}/gui-${state}-${w}x${h}.png"
  done
done

# HiDPI 2x: physical window 2560x1600, pixels_per_point 2 (1280x800 logical).
start_x 2560 1600
for state in "${STATES[@]}"; do
  capture_one "${state}" 2560 1600 2 "${OUT}/gui-${state}-1280x800@2x.png"
done

echo "done. files in ${OUT}:"
ls -lh "${OUT}"/gui-*-*.png
