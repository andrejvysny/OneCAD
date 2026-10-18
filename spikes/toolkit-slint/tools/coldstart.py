"""Launch-to-first-frame timing for the spike (stdlib only).

    python3 -I tools/coldstart.py [runs] [binary]

Each run spawns the binary with SPIKE_EXIT_AFTER_FIRST_FRAME=1 and reads its `[spike]` stderr
lines. The spawn time and the app's `uptime_ns` stamps use the same clock (CLOCK_UPTIME_RAW), so
spawn -> main and spawn -> first frame are directly comparable. The OS file cache is not purged
(that needs root), so only the first run after a build or reboot is close to cold.
"""

import re
import statistics
import subprocess
import sys
import time

runs = int(sys.argv[1]) if len(sys.argv) > 1 else 5
binary = sys.argv[2] if len(sys.argv) > 2 else "target/release/toolkit-slint"
env = {"SPIKE_EXIT_AFTER_FIRST_FRAME": "1", "SPIKE_WINDOW": "1521x921", "PATH": "/usr/bin:/bin"}
rows = []
for i in range(runs):
    t0 = time.clock_gettime_ns(time.CLOCK_UPTIME_RAW)
    p = subprocess.run([binary], env=env, capture_output=True, text=True, timeout=60)
    t_exit = time.clock_gettime_ns(time.CLOCK_UPTIME_RAW)
    log = p.stderr
    start = int(re.search(r"\[spike\] start uptime_ns=(\d+)", log).group(1))
    first = int(re.search(r"first_frame ms_since_main=[\d.]+ uptime_ns=(\d+)", log).group(1))
    window = float(re.search(r"window_created ms_since_main=([\d.]+)", log).group(1))
    load = float(re.search(r"load_ms=([\d.]+)", log).group(1))
    row = {
        "spawn_to_main": (start - t0) / 1e6,
        "fixture_load": load,
        "main_to_window": window,
        "main_to_first_frame": (first - start) / 1e6,
        "spawn_to_first_frame": (first - t0) / 1e6,
        "spawn_to_exit": (t_exit - t0) / 1e6,
    }
    rows.append(row)
    print(f"run {i + 1}: " + " ".join(f"{k}={v:.1f}ms" for k, v in row.items()), flush=True)
    time.sleep(1.0)
for k in rows[0]:
    vals = [r[k] for r in rows]
    print(f"{k}: first={vals[0]:.1f} median={statistics.median(vals):.1f} "
          f"min={min(vals):.1f} max={max(vals):.1f} ms")
