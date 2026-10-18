"""Count tapped key/button events not posted by this test (keys tool pids, ta-helper).
usage: python3 -I foreign.py <log.steps> <log.tap>"""
import re
import sys

own = set(re.findall(r"pid=(\d+)", open(sys.argv[1]).read()))
foreign = []
for line in open(sys.argv[2]):
    m = re.search(r"src=(\S+)", line)
    if not m or not line.startswith("[tap]"):
        continue
    src = m.group(1)
    if src in ("keys", "ta-helper") or (src.startswith("pid") and src[3:] in own):
        continue
    foreign.append(line.strip())
print(f"foreign_events={len(foreign)}")
for f in foreign[:20]:
    print(f)
