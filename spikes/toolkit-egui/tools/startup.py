"""Launch-to-first-frame timing. t0 = CLOCK_UPTIME_RAW just before spawn; the app prints
`[metric] first_frame_prepare ... uptime_ns=` on the same clock. Kills the app after.

usage: python3 -I startup.py <binary> <runs>
"""
import os
import re
import subprocess
import sys
import time

binary, runs = sys.argv[1], int(sys.argv[2])
env = dict(os.environ, SPIKE_WINDOW_POS="100,120")
for i in range(runs):
    t0 = time.clock_gettime_ns(time.CLOCK_UPTIME_RAW)
    p = subprocess.Popen([binary], stderr=subprocess.PIPE, text=True, env=env)
    vals = {}
    for line in p.stderr:
        m = re.search(r"\[metric\] (main|app_new_enter|app_created|first_frame_prepare)\b.*uptime_ns=(\d+)", line)
        if m:
            vals[m.group(1)] = int(m.group(2))
        if "rss after_first_frame" in line:
            vals["rss"] = line.strip().split("rss after_first_frame ")[1]
            break
    p.kill()
    p.wait()
    ms = lambda k: (vals[k] - t0) / 1e6
    print(f"run={i} spawn_to_main_ms={ms('main'):.1f} spawn_to_app_new_ms={ms('app_new_enter'):.1f} "
          f"spawn_to_app_created_ms={ms('app_created'):.1f} "
          f"spawn_to_first_frame_ms={ms('first_frame_prepare'):.1f} {vals.get('rss', '')}")
    time.sleep(1.0)
