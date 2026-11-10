#!/usr/bin/env bash
# Capture the Liquid Glass GUI in light and dark at 1024x700, 1280x800,
# 1920x1080, plus 1280x800 @2x.
set -euo pipefail

BIN="${BIN:-$(cd "$(dirname "$0")/.." && pwd)/target/release/framewipe-gui}"
if [[ ! -x "$BIN" ]]; then
  BIN="$(cd "$(dirname "$0")/.." && pwd)/target/debug/framewipe-gui"
fi
OUT="${OUT:-/workspace/frame/shots/liquid-glass}"
DISPLAY_NUM="${DISPLAY_NUM:-44}"
export DISPLAY=":${DISPLAY_NUM}"
export LIBGL_ALWAYS_SOFTWARE="${LIBGL_ALWAYS_SOFTWARE:-1}"
export GDK_SCALE=1
export GDK_DPI_SCALE=1
export QT_AUTO_SCREEN_SCALE_FACTOR=0
export QT_SCALE_FACTOR=1
export WINIT_X11_SCALE_FACTOR=1
export LANG="${LANG:-C}"
export LC_ALL="${LC_ALL:-C}"
export FRAME_WIPE_FORBID_REAL_DISKS=1

mkdir -p "$OUT"

STATES=(empty error list selected confirm countdown progress done failed)
APPEARANCES=(dark light)
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
  rm -f "/tmp/.X${DISPLAY_NUM}-lock" "/tmp/.X11-unix/X${DISPLAY_NUM}"
  Xvfb ":${DISPLAY_NUM}" -screen 0 "${w}x${h}x24" -dpi 96 -ac +extension GLX +render -noreset -nocursor >/tmp/fw-lg-xvfb.log 2>&1 &
  XPID=$!
  local i
  for i in $(seq 1 50); do
    if xdpyinfo -display ":${DISPLAY_NUM}" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  echo "Xvfb failed to start on :${DISPLAY_NUM}" >&2
  cat /tmp/fw-lg-xvfb.log >&2 || true
  return 1
}

window_id() {
  local id
  id="$(xdotool search --sync --all --onlyvisible --class framewipe 2>/dev/null | tail -n1 || true)"
  if [[ -z "${id}" ]]; then
    id="$(xdotool search --sync --all --onlyvisible --name "Frame Wipe" 2>/dev/null | tail -n1 || true)"
  fi
  if [[ -z "${id}" ]]; then
    id="$(xwininfo -root -tree | awk '/(has no name)|("Frame Wipe")|framewipe/{id=$1} END{print id}' | tr -d ':')"
  fi
  echo "${id}"
}

lock_window() {
  local phys_w="$1" phys_h="$2"
  local id="" i geom
  for i in $(seq 1 40); do
    id="$(window_id)"
    if [[ -n "${id}" ]]; then
      break
    fi
    sleep 0.1
  done
  if [[ -z "${id}" ]]; then
    echo "could not find GUI window" >&2
    xwininfo -root -tree >&2 || true
    return 1
  fi
  xdotool windowmove "${id}" 0 0
  xdotool windowsize "${id}" "${phys_w}" "${phys_h}"
  for i in $(seq 1 40); do
    geom="$(xwininfo -id "${id}" | awk '/-geometry/{print $2}')"
    if [[ "${geom}" == "${phys_w}x${phys_h}+0+0" || "${geom}" == "${phys_w}x${phys_h}+0-0" ]]; then
      echo "window ${id} geometry ${geom}"
      return 0
    fi
    xdotool windowmove "${id}" 0 0 >/dev/null 2>&1 || true
    xdotool windowsize "${id}" "${phys_w}" "${phys_h}" >/dev/null 2>&1 || true
    sleep 0.1
  done
  echo "window geometry mismatch: got ${geom}, expected ${phys_w}x${phys_h}+0+0" >&2
  xwininfo -id "${id}" >&2 || true
  return 1
}

capture_one() {
  local state="$1" logical_w="$2" logical_h="$3" scale="$4" appearance="$5" dest="$6"
  local phys_w phys_h
  phys_w="$(python3 -c "print(int(round(${logical_w} * ${scale})))")"
  phys_h="$(python3 -c "print(int(round(${logical_h} * ${scale})))")"
  local ready="/tmp/fw-lg-ready-$$"
  rm -f "${ready}"
  "${BIN}" --demo --demo-state "${state}" --appearance "${appearance}" \
    --width "${logical_w}" --height "${logical_h}" --scale "${scale}" \
    --undecorated --ready-file "${ready}" >/tmp/fw-lg-gui.log 2>&1 &
  GPID=$!
  local i
  for i in $(seq 1 100); do
    if [[ -f "${ready}" ]]; then
      break
    fi
    if ! kill -0 "${GPID}" 2>/dev/null; then
      echo "GUI exited before ready (${appearance} ${state} ${logical_w}x${logical_h})" >&2
      cat /tmp/fw-lg-gui.log >&2 || true
      GPID=""
      return 1
    fi
    sleep 0.1
  done
  if [[ ! -f "${ready}" ]]; then
    echo "timed out waiting for ready file (${appearance} ${state} ${logical_w}x${logical_h})" >&2
    cat /tmp/fw-lg-gui.log >&2 || true
    kill "${GPID}" 2>/dev/null || true
    wait "${GPID}" 2>/dev/null || true
    GPID=""
    return 1
  fi
  cat "${ready}"
  lock_window "${phys_w}" "${phys_h}"
  # Let glass animations settle (sheet rise, ring lerp).
  sleep 0.8
  xdotool mousemove -- 10000 10000 >/dev/null 2>&1 || true
  ffmpeg -y -hide_banner -loglevel error \
    -f x11grab -draw_mouse 0 -video_size "${phys_w}x${phys_h}" -i ":${DISPLAY_NUM}" \
    -frames:v 1 "${dest}"
  python3 - "${dest}" "${phys_w}" "${phys_h}" <<'PY'
import sys
from pathlib import Path
path, w, h = Path(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3])
try:
    from PIL import Image
except ImportError:
    import struct
    data = path.read_bytes()
    iw, ih = struct.unpack(">II", data[16:24])
else:
    im = Image.open(path)
    iw, ih = im.size
    im.convert("RGB").save(path, optimize=True)
if (iw, ih) != (w, h):
    raise SystemExit(f"{path} is {iw}x{ih}, expected {w}x{h}")
print(f"verified {path} {iw}x{ih}")
PY
  kill "${GPID}" 2>/dev/null || true
  wait "${GPID}" 2>/dev/null || true
  GPID=""
  echo "wrote ${dest} ($(wc -c < "${dest}") bytes)"
}

if [[ ! -x "${BIN}" ]]; then
  echo "missing binary: ${BIN}" >&2
  exit 1
fi

echo "using ${BIN}"

# Fast path: only 1280 if FIRST=1
if [[ "${FIRST:-}" == "1" ]]; then
  start_x 1280 800
  for appearance in "${APPEARANCES[@]}"; do
    for state in "${STATES[@]}"; do
      capture_one "${state}" 1280 800 1 "${appearance}" "${OUT}/gui-${state}-1280x800-${appearance}.png"
    done
  done
  echo "first pass done"
  exit 0
fi

for size in "${SIZES[@]}"; do
  w="${size%x*}"
  h="${size#*x}"
  start_x "${w}" "${h}"
  for appearance in "${APPEARANCES[@]}"; do
    for state in "${STATES[@]}"; do
      capture_one "${state}" "${w}" "${h}" 1 "${appearance}" "${OUT}/gui-${state}-${w}x${h}-${appearance}.png"
    done
  done
done

start_x 2560 1600
for appearance in "${APPEARANCES[@]}"; do
  for state in "${STATES[@]}"; do
    capture_one "${state}" 1280 800 2 "${appearance}" "${OUT}/gui-${state}-1280x800@2x-${appearance}.png"
  done
done

echo "done. files in ${OUT}:"
ls -lh "${OUT}"/gui-*.png
