"""Split photon.swift's event->display latency at the app's frame `prepare`.

usage: python3 -I photon_join.py <photon.log> <app.log>
photon.log: `[photon] post_ns=.. display_ns=.. ms=..` lines (one per matched drag step).
app.log:    `[latency] x=.. y=.. prepare_ns=..` lines (app run with SPIKE_LATENCY=1).
All stamps are CLOCK_UPTIME_RAW ns on one host. For each step, the first prepare after the
post is the frame that consumed it (steps are 100+ ms apart, frames are on-demand).
"""
import re
import statistics
import sys

steps = [tuple(map(int, m.groups())) for m in re.finditer(r"post_ns=(\d+) display_ns=(\d+)", open(sys.argv[1]).read())]
prep = sorted(int(m.group(1)) for m in re.finditer(r"\[latency\] x=[\d.-]+ y=[\d.-]+ prepare_ns=(\d+)", open(sys.argv[2]).read()))
a, b = [], []
for post, disp in steps:
    hit = next((t for t in prep if t >= post), None)
    if hit is None or hit > disp:
        continue
    a.append((hit - post) / 1e6)
    b.append((disp - hit) / 1e6)


def stats(name, v):
    s = sorted(v)
    q = lambda p: s[round((len(s) - 1) * p)]
    print(f"{name} n={len(s)} mean={statistics.mean(s):.2f} p50={q(0.5):.2f} p95={q(0.95):.2f} min={s[0]:.2f} max={s[-1]:.2f}")


print(f"steps={len(steps)} joined={len(a)}")
if a:
    stats("event_to_prepare_ms", a)
    stats("prepare_to_display_ms", b)
