"""Joins tools/inject.swift output with the app's `[spike]` log (stdlib only).

    python3 -I tools/latency.py <inject-output.txt> <app.log>

The app runs with SPIKE_LOG_FRAMES=1. Each logged scene frame carries its AfterRendering time P
(just before Slint calls `present()`) and the latency L from the raw winit event it is attributed
to, which is the oldest input not yet shown. So R = P - L is that event's arrival in the app.
The nearest injected post at or before R gives the OS delivery time (R - T) and the full
post-to-present time (P - T). All stamps are CLOCK_UPTIME_RAW.
"""

import bisect
import re
import statistics
import sys

posts = sorted(int(m) for m in re.findall(r"uptime_ns=(\d+)", open(sys.argv[1]).read()))
frames = [
    (int(p), float(l))
    for l, p in re.findall(r"input_to_present_ms=([\d.]+) uptime_ns=(\d+)", open(sys.argv[2]).read())
]
lo, hi = posts[0], posts[-1] + 200_000_000
delivery, app, total = [], [], []
for p, l in frames:
    r = p - int(l * 1e6)
    if not lo <= r <= hi:
        continue
    i = bisect.bisect_right(posts, r) - 1
    if i < 0:
        continue
    t = posts[i]
    delivery.append((r - t) / 1e6)
    app.append(l)
    total.append((p - t) / 1e6)


def summary(v: list[float]) -> str:
    v = sorted(v)
    q = lambda f: v[round((len(v) - 1) * f)]
    return (f"n={len(v)} mean={statistics.mean(v):.2f} p50={q(0.5):.2f} p95={q(0.95):.2f} "
            f"max={v[-1]:.2f} ms")


print(f"posts={len(posts)} frames_in_window={len(total)}")
print("os_delivery (post -> winit event)      ", summary(delivery))
print("app (winit event -> before present)    ", summary(app))
print("post_to_present (post -> before present)", summary(total))
