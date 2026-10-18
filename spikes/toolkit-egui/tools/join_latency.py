"""Join inject_drag output with the app's [latency] lines (both CLOCK_UPTIME_RAW ns).

usage: python3 -I join_latency.py <inject.log> <app.log> <window_x_points>
For each injected drag event, find the first app frame whose consumed pointer x equals the
event's window-local x and whose prepare time is after the post time.
"""
import re
import statistics
import sys

inj = [tuple(map(float, m.groups())) for m in re.finditer(r"\[inject\] i=\d+ x=([\d.-]+) y=[\d.-]+ post_ns=(\d+)", open(sys.argv[1]).read())]
app = [tuple(map(float, m.groups())) for m in re.finditer(r"\[latency\] x=([\d.-]+) y=[\d.-]+ prepare_ns=(\d+)", open(sys.argv[2]).read())]
wx = float(sys.argv[3])
lat = []
for gx, post in inj:
    lx = gx - wx
    hit = next((t for x, t in app if abs(x - lx) < 0.6 and t >= post), None)
    if hit is not None:
        lat.append((hit - post) / 1e6)
print(f"injected={len(inj)} matched={len(lat)}")
if lat:
    s = sorted(lat)
    q = lambda p: s[round((len(s) - 1) * p)]
    print(f"input_to_prepare_ms mean={statistics.mean(s):.2f} p50={q(0.5):.2f} p95={q(0.95):.2f} min={s[0]:.2f} max={s[-1]:.2f}")
