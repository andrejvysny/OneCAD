#!/bin/zsh
# Real-input latency run: launches the spike, posts CGEvent drags (orbit) and trackpad-style pixel
# scrolls (pan) into its viewport, then joins the timestamps with tools/latency.py.
#   HELPER=<tauri-agent helper> INJECT=<compiled tools/inject.swift> tools/latency-run.sh [binary] [outdir]
# HELPER is tools/tauri-agent/src/platform/macos/helper/main.swift built with
#   swiftc -O -framework Cocoa -framework Carbon main.swift -o tauri-agent-helper
# This moves the real mouse pointer for a few seconds.
set -euo pipefail
cd "${0:A:h}/.."
BIN=${1:-target/release/toolkit-slint}
OUT=${2:-/tmp}
: ${HELPER:?} ${INJECT:?}
SPIKE_WINDOW=1521x921 SPIKE_WINDOW_POS=${SPIKE_WINDOW_POS:-1030,60} SPIKE_LOG_FRAMES=1 SPIKE_LOG_INPUT=1 $BIN > $OUT/lat-app.log 2>&1 &
APP=$!
sleep 2.5
read X Y <<< $(printf '{"id":1,"verb":"windows","pid":%s}\n{"id":2,"verb":"focus","pid":%s}\n' $APP $APP \
  | $HELPER 2>/dev/null | head -1 | python3 -I -c 'import json,sys; b=json.load(sys.stdin)["result"]["windows"][0]["bounds"]; print(b["x"], b["y"])')
# CGEvents go to whatever window is under the pointer: refuse to post unless the spike is in front.
front() {
  printf '{"id":1,"verb":"focus","pid":%s}\n{"id":2,"verb":"frontmost","pid":%s}\n' $APP $APP \
    | $HELPER 2>/dev/null | tail -1 | grep -q '"frontmost":true' || { echo "spike not frontmost"; kill $APP; exit 1; }
}
# Viewport-relative points (the viewport spans window x 220..1263, y 44..887).
front
$INJECT drag $((X + 650)) $((Y + 650)) $((X + 950)) $((Y + 550)) 150 8 > $OUT/lat-drag.txt
sleep 0.5
front
$INJECT scroll $((X + 650)) $((Y + 500)) 120 -6 8 > $OUT/lat-scroll.txt
sleep 0.5
kill $APP
wait $APP 2>/dev/null || true
echo "== drag (orbit), posts every 8 ms"
python3 -I tools/latency.py $OUT/lat-drag.txt $OUT/lat-app.log
echo "== trackpad pixel scroll (pan), posts every 8 ms"
python3 -I tools/latency.py $OUT/lat-scroll.txt $OUT/lat-app.log
