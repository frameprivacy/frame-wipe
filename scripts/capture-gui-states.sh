#!/usr/bin/env bash
# Capture every GUI demo state at 1024x700, 1280x800, and 1920x1080 logical
# pixels under a private Xvfb, plus a 2x HiDPI set at 1280x800 logical
# (2560x1600 physical).
#
# --width/--height passed to the app are LOGICAL sizes. The Xvfb screen and
# ffmpeg grab are physical (logical * scale). The window is moved to 0,0 and
# resized to the physical size so each capture is a real framebuffer of that
# size, not a 1024-sized window sitting on a larger black screen.
set -euo pipefail

BIN="${BIN:-$(cd "$(dirname "$0")/.." && pwd)/target/release/framewipe-gui}"
OUT="${OUT:-/workspace/frame/shots/zero-app}"
DISPLAY_NUM="${DISPLAY_NUM:-43}"
export DISPLAY=":${DISPLAY_NUM}"
export LIBGL_ALWAYS_SOFTWARE="${LIBGL_ALWAYS_SOFTWARE:-1}"
# Keep toolkit scale from overriding the requested pixels_per_point.
export GDK_SCALE=1
export GDK_DPI_SCALE=1
export QT_AUTO_SCREEN_SCALE_FACTOR=0
export QT_SCALE_FACTOR=1
export WINIT_X11_SCALE_FACTOR=1

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
  rm -f "/tmp/.X${DISPLAY_NUM}-lock" "/tmp/.X11-unix/X${DISPLAY_NUM}"
  Xvfb ":${DISPLAY_NUM}" -screen 0 "${w}x${h}x24" -dpi 96 -ac +extension GLX +render -noreset -nocursor >/tmp/fw-xvfb.log 2>&1 &
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

window_id() {
  # The undecorated eframe window; pick the mapped one that is not the root.
  local id
  id="$(xdotool search --sync --all --onlyvisible --class framewipe 2>/dev/null | tail -n1 || true)"
  if [[ -z "${id}" ]]; then
    id="$(xdotool search --sync --all --onlyvisible --name "Frame Wipe" 2>/dev/null | tail -n1 || true)"
  fi
  if [[ -z "${id}" ]]; then
    # Fallback: first non-root mapped window on this display.
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
  # Wait until the reported geometry matches. Some WMs/winit updates lag a frame.
  for i in $(seq 1 40); do
    geom="$(xwininfo -id "${id}" | awk '/-geometry/{print $2}')"
    # geometry looks like 1280x800+0+0 or 1280x800+0+0
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
  local state="$1" logical_w="$2" logical_h="$3" scale="$4" dest="$5"
  local phys_w phys_h
  phys_w="$(python3 -c "print(int(round(${logical_w} * ${scale})))")"
  phys_h="$(python3 -c "print(int(round(${logical_h} * ${scale})))")"
  local ready="/tmp/fw-gui-ready-$$"
  rm -f "${ready}"
  "${BIN}" --demo --demo-state "${state}" \
    --width "${logical_w}" --height "${logical_h}" --scale "${scale}" \
    --undecorated --ready-file "${ready}" >/tmp/fw-gui.log 2>&1 &
  GPID=$!
  local i
  for i in $(seq 1 100); do
    if [[ -f "${ready}" ]]; then
      break
    fi
    if ! kill -0 "${GPID}" 2>/dev/null; then
      echo "GUI exited before ready (${state} ${logical_w}x${logical_h} scale=${scale})" >&2
      cat /tmp/fw-gui.log >&2 || true
      GPID=""
      return 1
    fi
    sleep 0.1
  done
  if [[ ! -f "${ready}" ]]; then
    echo "timed out waiting for ready file (${state} ${logical_w}x${logical_h})" >&2
    cat /tmp/fw-gui.log >&2 || true
    kill "${GPID}" 2>/dev/null || true
    wait "${GPID}" 2>/dev/null || true
    GPID=""
    return 1
  fi
  echo "ready (${state} logical=${logical_w}x${logical_h} scale=${scale} physical=${phys_w}x${phys_h}):"
  cat "${ready}"
  logical_line=$(grep -E "^logical=" "${ready}" || true)
  if [[ "${logical_line}" != "logical=${logical_w}x${logical_h}" ]]; then
    echo "logical size mismatch: ${logical_line} (expected logical=${logical_w}x${logical_h})" >&2
    kill "${GPID}" 2>/dev/null || true
    wait "${GPID}" 2>/dev/null || true
    GPID=""
    return 1
  fi
  ppp_line=$(grep -E "^ppp=" "${ready}" || true)
  if [[ "${ppp_line}" != "ppp=${scale}" && "${ppp_line}" != "ppp=${scale}.0" && "${ppp_line}" != "ppp=${scale}.00" ]]; then
    # Accept 1 and 1.0
    ppp_val="${ppp_line#ppp=}"
    match="$(python3 -c "print(abs(float('${ppp_val}') - float('${scale}')) < 0.01)")"
    if [[ "${match}" != "True" ]]; then
      echo "scale mismatch: ${ppp_line} (expected ppp=${scale})" >&2
      kill "${GPID}" 2>/dev/null || true
      wait "${GPID}" 2>/dev/null || true
      GPID=""
      return 1
    fi
  fi
  lock_window "${phys_w}" "${phys_h}"
  # Let eframe present a composed frame at the locked size.
  sleep 0.5
  xdotool mousemove -- 10000 10000 >/dev/null 2>&1 || true
  ffmpeg -y -hide_banner -loglevel error \
    -f x11grab -draw_mouse 0 -video_size "${phys_w}x${phys_h}" -i ":${DISPLAY_NUM}" \
    -frames:v 1 "${dest}"
  # Verify the PNG is exactly the physical size.
  python3 - "${dest}" "${phys_w}" "${phys_h}" <<'PY'
import sys
from pathlib import Path
path, w, h = Path(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3])
try:
    from PIL import Image
except ImportError:
    import struct
    data = path.read_bytes()
    # PNG IHDR: width/height at bytes 16-23
    iw, ih = struct.unpack(">II", data[16:24])
else:
    im = Image.open(path)
    iw, ih = im.size
    # ffmpeg writes a pHYs chunk with a 0:1 aspect that makes some viewers
    # rescale the image; re-save without it.
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

for size in "${SIZES[@]}"; do
  w="${size%x*}"
  h="${size#*x}"
  start_x "${w}" "${h}"
  for state in "${STATES[@]}"; do
    capture_one "${state}" "${w}" "${h}" 1 "${OUT}/gui-${state}-${w}x${h}.png"
  done
done

# HiDPI 2x: logical 1280x800, physical 2560x1600, pixels_per_point 2.
start_x 2560 1600
for state in "${STATES[@]}"; do
  capture_one "${state}" 1280 800 2 "${OUT}/gui-${state}-1280x800@2x.png"
done

# Cross-size sanity: 1280 and 1920 content must actually be wider than 1024,
# so a padded 1024 window cannot pass as a larger capture.
python3 - "${OUT}" <<'PY'
import sys
from pathlib import Path
out = Path(sys.argv[1])
try:
    from PIL import Image
except ImportError:
    print("pillow missing, skip content-width check")
    raise SystemExit(0)

def right_edge(path, threshold=20):
    im = Image.open(path).convert("RGB")
    w, h = im.size
    px = im.load()
    right = 0
    # Ignore the header buttons strip (top 56 px) and use mid-window content.
    for y in range(60, min(h - 40, 400)):
        for x in range(w - 1, -1, -1):
            r, g, b = px[x, y]
            if r > threshold or g > threshold or b > threshold:
                if x > right:
                    right = x
                break
    return w, right

def left_card_width(path, threshold=16):
    # First device card lives in the left column, roughly y=100-200.
    im = Image.open(path).convert("RGB")
    w, h = im.size
    px = im.load()
    max_x = 0
    for y in range(90, min(h, 220)):
        for x in range(0, min(w, 800)):
            r, g, b = px[x, y]
            if r > threshold or g > threshold or b > threshold:
                if x > max_x:
                    max_x = x
        # stop at the gutter: after the card there is a black gap before the
        # right panel. Detect by finding the first long black run after content.
    # Simpler: measure the right edge of content in the upper-left quadrant
    # by finding the first x where a vertical slice is almost all background
    # after we have already seen a card.
    edges = []
    for y in range(100, 200):
        run_bg = 0
        seen = False
        edge = 0
        for x in range(0, min(w // 2 + 80, w)):
            r, g, b = px[x, y]
            if r > threshold or g > threshold or b > threshold:
                seen = True
                edge = x
                run_bg = 0
            elif seen:
                run_bg += 1
                if run_bg > 12:
                    break
        if edge:
            edges.append(edge)
    return max(edges) if edges else 0

checks = [
    ("gui-list-1024x700.png", "gui-list-1280x800.png", 80),
    ("gui-list-1280x800.png", "gui-list-1920x1080.png", 80),
    ("gui-selected-1024x700.png", "gui-selected-1280x800.png", 80),
]
for a, b, min_delta in checks:
    pa, pb = out / a, out / b
    wa, ra = right_edge(pa)
    wb, rb = right_edge(pb)
    ca, cb = left_card_width(pa), left_card_width(pb)
    print(f"{a}: img={wa} content_right={ra} left_card={ca}")
    print(f"{b}: img={wb} content_right={rb} left_card={cb}")
    if rb - ra < min_delta:
        raise SystemExit(
            f"{b} content right edge {rb} is not clearly wider than {a} ({ra}); "
            "the larger capture may be a padded smaller window"
        )
print("size differentiation ok")
PY

echo "done. files in ${OUT}:"
ls -lh "${OUT}"/gui-*-*.png
