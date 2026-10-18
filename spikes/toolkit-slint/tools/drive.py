"""Real-input scenario driver for the spike, through the tauri-agent macOS helper (stdlib only).

    HELPER=<tauri-agent helper> python3 -I tools/drive.py <pid> <app.log> <shots dir> [scenario]

scenario: `menu` (default) runs the dimension-input and undo-routing checks, `ime` the input
method checks. Keys are real virtual-key CGEvents (`press`), so they pass through AppKit's
key-equivalent and input-method machinery; `type` posts Unicode strings and bypasses the IME.
Every step refuses to run unless the spike window is frontmost, since CGEvents go to whatever
window is under the pointer. Prints each step and the app log lines it produced.
UNDO_KEY names the physical key that types "z" on the active layout ("y" on QWERTZ layouts such
as Slovak or German), because `press` posts US-ANSI virtual key codes.
"""

import json
import os
import subprocess
import sys
import time

pid, log_path, shots = int(sys.argv[1]), sys.argv[2], sys.argv[3]
scenario = sys.argv[4] if len(sys.argv) > 4 else "menu"
undo_key = os.environ.get("UNDO_KEY", "z")
helper = subprocess.Popen([os.environ["HELPER"]], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                          stderr=subprocess.DEVNULL, text=True)
seq = 0
log_pos = os.path.getsize(log_path)


def call(verb: str, **args):
    global seq
    seq += 1
    helper.stdin.write(json.dumps({"id": seq, "verb": verb, "pid": pid, **args}) + "\n")
    helper.stdin.flush()
    reply = json.loads(helper.stdout.readline())
    if not reply.get("ok"):
        raise SystemExit(f"{verb} failed: {reply}")
    return reply["result"]


def new_log_lines() -> list[str]:
    global log_pos
    with open(log_path) as f:
        f.seek(log_pos)
        text = f.read()
        log_pos = f.tell()
    keep = ("raw cursor", "raw modifiers", "vp-hover", "input vp-")
    return [l for l in text.splitlines() if not any(k in l for k in keep)]


def step(name: str, fn, wait: float = 0.4, refocus: bool = True):
    # `refocus=False` inside an IME composition: re-activating the app mid-composition can move
    # key status to the input method's in-process candidate window.
    if refocus:
        call("focus")
    if not call("frontmost")["frontmost"]:
        raise SystemExit("spike window is not frontmost; refusing to post input")
    fn()
    time.sleep(wait)
    print(f"--- {name}")
    for line in new_log_lines():
        print("   ", line)


def window():
    # The IME candidate panel is an in-process window too; pick the app window by its title.
    return next(w for w in call("windows")["windows"] if w.get("name", "").startswith("OneCAD"))


def shot(name: str):
    subprocess.run(["screencapture", "-x", "-o", "-l", str(window()["windowId"]),
                    f"{shots}/{name}"], check=True)
    print(f"    screenshot {shots}/{name}")


def shot_region(name: str):
    # Screen region around the field, so in-process IME panels (separate windows) are included.
    b = field()["bounds"]
    r = f"{int(b['x']) - 120},{int(b['y']) - 40},520,320"
    subprocess.run(["screencapture", "-x", "-R", r, f"{shots}/{name}"], check=True)
    print(f"    screenshot {shots}/{name} (screen region {r})")


def field():
    nodes = call("ax_find", role="AXTextField")["nodes"]
    return nodes[0]


def click_field():
    b = field()["bounds"]
    call("click", button="left", x=b["x"] + b["w"] / 2, y=b["y"] + b["h"] / 2)


def click_select_all():
    # A click places the caret; Cmd+A (no menu item claims it) reaches the TextInput.
    click_field()
    time.sleep(0.3)
    call("press", key="a", mods=["cmd"])


def press(key: str, *mods: str):
    return lambda: call("press", key=key, mods=list(mods))


def menu_scenario():
    w = window()["bounds"]
    # Park the pointer on the title strip (no hover targets there) for the idle-frame count.
    step("park pointer, reset metrics (Cmd+Shift+R)",
         lambda: (call("move", x=w["x"] + 700, y=w["y"] + 20), call("press", key="r", mods=["cmd", "shift"])),
         wait=3.0)
    step("after 3 s idle: report frames (Cmd+Shift+R)", press("r", "cmd", "shift"))
    step("click dimension field, Cmd+A", click_select_all)
    step("type '10 mm + 2*3'", lambda: call("type", text="10 mm + 2*3"))
    shot("mac-dim-typing.png")
    step("Return commits", press("return"))
    print("    field value:", field().get("value"))
    step("Cmd+Z with field unfocused -> document undo", press(undo_key, "cmd"))
    print("    field value:", field().get("value"))
    step("Cmd+Shift+Z -> document redo", press(undo_key, "cmd", "shift"))
    print("    field value:", field().get("value"))
    step("click field, Cmd+A, type '5'", lambda: (click_select_all(), call("type", text="5")))
    print("    field value:", field().get("value"))
    step("Cmd+Z with field focused -> text-field undo", press(undo_key, "cmd"))
    print("    field value:", field().get("value"))
    step("Escape cancels", press("escape"))
    print("    field value:", field().get("value"))
    step("Cmd+D (Edit Dimension), type 'abc'",
         lambda: (call("press", key="d", mods=["cmd"]), time.sleep(0.3), call("type", text="abc")))
    shot("mac-dim-error.png")
    step("Return on an invalid expression", press("return"))
    step("Escape", press("escape"))
    row = call("ax_find", role="AXStaticText", title="Body 3")["nodes"][0]["bounds"]
    step("click tree row 'Body 3'",
         lambda: call("click", button="left", x=row["x"] + 60, y=row["y"] + row["h"] / 2))
    inspector = call("ax_find", role="AXStaticText", value="Triangles")["nodes"]
    print("    inspector rows found by AX:", len(inspector))
    shot("mac-selected.png")


def ime_scenario():
    step("click dimension field, Cmd+A", click_select_all)
    step("Japanese romaji: n i h o n", lambda: [call("press", key=k) for k in "nihon"])
    shot("mac-ime-preedit.png")
    step("Space (convert)", press("space"), wait=0.8, refocus=False)
    shot_region("mac-ime-candidate.png")
    step("Return (accept candidate)", press("return"), wait=0.6, refocus=False)
    print("    field value:", field().get("value"))
    step("Return (commit dimension)", press("return"), refocus=False)
    print("    field value:", field().get("value"))


def accent_scenario():
    # Dead keys on the Slovak layout: key code 24 ("=" in US-ANSI names) is acute, Shift+24 caron.
    step("click dimension field, Cmd+A", click_select_all)
    step("dead acute, then e", lambda: (call("press", key="="), time.sleep(0.2), call("press", key="e")))
    step("dead caron (Shift), then e",
         lambda: (call("press", key="=", mods=["shift"]), time.sleep(0.2), call("press", key="e")))
    print("    field value:", field().get("value"))
    shot("mac-accent.png")
    step("Escape", press("escape"))


{"menu": menu_scenario, "ime": ime_scenario, "accent": accent_scenario}[scenario]()
helper.stdin.close()
