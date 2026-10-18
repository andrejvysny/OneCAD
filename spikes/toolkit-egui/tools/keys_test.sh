#!/bin/zsh
# Real-input test of the dimension field, the native Edit menu routing and dead-key IME,
# driven through the tauri-agent CGEvent/AX helper (tools/tauri-agent/.../helper/main.swift).
# usage: keys_test.sh <helper> <keys-tool> <app-binary> <log> [edit|ime|cjk|all]
# Keys go through tools/keys.swift (40 ms hold): the helper's zero-gap `press` was observed to
# post nothing for unmodified keys (an event tap saw only the first, modified one).
set -u
H=$1; K=$2; BIN=$3; LOG=$4; PHASE=${5:-all}
# Another agent shares this desktop (toolkit-slint): wait for a quiet window.
for i in $(seq 1 120); do
  if ! pgrep -x toolkit-slint > /dev/null; then break; fi
  sleep 2
done
pkill -x toolkit-egui 2>/dev/null; sleep 0.5; : > "$LOG.steps"
pbpaste > "$LOG.clip"   # the copy/paste step overwrites the pasteboard; restored at the end
trap 'pbcopy < "$LOG.clip"; rm -f "$LOG.clip"' EXIT
pbcopy <<< "2*21 mm"
(SPIKE_WINDOW_POS=100,120 SPIKE_INPUT_LOG=1 "$BIN" > "$LOG" 2>&1 &)
# Every key/button event on the session, with its posting process (foreign-input detector).
("$(dirname "$K")/keytap" 90 > "$LOG.tap" 2>&1 &)
sleep 2.5
P=$(pgrep -x toolkit-egui | head -1)
# The helper announces "ready ... pid=N" on stderr; keep it so foreign.py can attribute events.
send() { print -r -- "$1" | "$H" 2>> "$LOG.steps"; }
step() { print -r -- "[step] $1 uptime_ns=$(python3 -c "import time;print(time.clock_gettime_ns(time.CLOCK_UPTIME_RAW))")" >> "$LOG.steps"; }
send "{\"id\":0,\"verb\":\"focus\",\"pid\":$P}" > /dev/null
# Field bounds from the AX tree (the tauri-agent lane's own view of the window).
F=$(send "{\"id\":1,\"verb\":\"ax_find\",\"pid\":$P,\"role\":\"AXTextField\"}" | python3 -c "import json,sys; b=json.load(sys.stdin)['result']['nodes'][0]['bounds']; print(int(b['x']+b['w']/2), int(b['y']+b['h']/2))")
FX=${F% *}; FY=${F#* }
WID=$(send "{\"id\":2,\"verb\":\"windows\",\"pid\":$P}" | python3 -c "import json,sys; print(json.load(sys.stdin)['result']['windows'][0]['windowId'])")
shot() { screencapture -o -x -l $WID "$LOG.$1.png"; print -r -- "[shot] $LOG.$1.png" >> "$LOG.steps"; }
# The desktop is shared (another agent drives its own window): re-activate the app and verify it
# is frontmost before every synthetic event, so no keystroke lands in someone else's window.
ensure() {
  for i in 1 2 3 4 5; do
    send "{\"id\":8,\"verb\":\"focus\",\"pid\":$P}" > /dev/null
    sleep 0.15
    send "{\"id\":8,\"verb\":\"frontmost\",\"pid\":$P}" | grep -q '"frontmost":true' && return 0
  done
  print -r -- "[abort] app not frontmost" >> "$LOG.steps"; pkill -x toolkit-egui; exit 3
}
# After each event: if the app lost the foreground meanwhile, the step is void (flagged).
after() { send "{\"id\":8,\"verb\":\"frontmost\",\"pid\":$P}" | grep -q '"frontmost":true' || print -r -- "[focus-lost] during: $1" >> "$LOG.steps"; }
typeset -A CODE=(z 6 y 16 a 0 c 8 v 9 e 14 = 24 return 36 escape 53 end 119)
key() {
  ensure
  local m=""
  [[ $2 == *command* ]] && m+=c; [[ $2 == *shift* ]] && m+=s; [[ $2 == *option* ]] && m+=o
  "$K" "${CODE[$1]}${m:+:$m}" >> "$LOG.steps"; sleep 0.25; after "key $1"
}
typ() { ensure; send "{\"id\":9,\"verb\":\"type\",\"text\":\"$1\",\"perCharMs\":25}" > /dev/null; sleep 0.3; after "type $1"; }
# A real click has tens of ms between press and release; the helper's zero-gap `click` loses
# the release in winit, so press and release are posted separately.
click() {
  ensure
  # egui hit-tests a press against the previous frame's hover, so a real pointer arrives first.
  send "{\"id\":9,\"verb\":\"move\",\"x\":$FX,\"y\":$FY}" > /dev/null; sleep 0.1
  send "{\"id\":9,\"verb\":\"down\",\"button\":\"left\",\"x\":$FX,\"y\":$FY}" > /dev/null; sleep 0.08
  send "{\"id\":9,\"verb\":\"up\",\"button\":\"left\",\"x\":$FX,\"y\":$FY}" > /dev/null; sleep 0.4; after click
}

# Layout: this Mac uses Slovak QWERTZ, where the key printing Z is ANSI kVK_ANSI_Y, so the
# helper's "y" is the user's Cmd+Z. Menu key equivalents match the logical character.
if [[ $PHASE == edit || $PHASE == all ]]; then
step "0 ANSI-Z position (prints Y on QWERTZ) with Cmd"; key z '["command"]'
step "1 Cmd+Z, field not focused"; key y '["command"]'
step "2 click field, Cmd+A, type expression, Enter"; click; key a '["command"]'; typ "10 mm + 2*3"; key return '[]'
step "3 Cmd+Z, field not focused (document undo)"; key y '["command"]'
step "4 Shift+Cmd+Z (document redo)"; key y '["command","shift"]'
step "5 click field, type 9, then Cmd+Z (text undo)"; click; key end '[]'; typ "9"; key y '["command"]'
step "6 Cmd+A, Cmd+V pastes the seeded '2*21 mm', Enter commits"; key a '["command"]'; key v '["command"]'; key return '[]'
step "6b click field, Cmd+A, Cmd+C copies the field text to the pasteboard"; click; key a '["command"]'; key c '["command"]'
print -r -- "[pasteboard] after copy: $(pbpaste)" >> "$LOG.steps"
step "6c Cmd+Z after commit, field not focused (document undo to the previous width)"; key escape '[]'; key y '["command"]'
fi
if [[ $PHASE == ime || $PHASE == all ]]; then
step "7 Escape cancels"; key escape '[]'
step "8 click field, Cmd+A, dead key (Slovak =, acute), shot while composing, then e"; click; key a '["command"]'; key = '[]'; shot composing; key e '[]'; shot composed
step "9 Option+e dead key then e"; key e '["option"]'; key e '[]'
step "10 Escape"; key escape '[]'
step "11 bad expression then Enter keeps focus"; click; key a '["command"]'; typ "10 parsecs"; key return '[]'; key escape '[]'
fi
if [[ $PHASE == cjk || $PHASE == all ]]; then
LANG=en_US.UTF-8 pbcopy <<< "東京 10 mm"
step "12 click field, Cmd+A, Cmd+V pastes CJK text (system-font fallback), shot, Escape"; click; key a '["command"]'; key v '["command"]'; shot cjk; key escape '[]'
fi
sleep 0.3
pkill -x toolkit-egui; pkill -x keytap
