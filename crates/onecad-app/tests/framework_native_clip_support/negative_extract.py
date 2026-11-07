"""Reproduce private negative-side pipe inputs with the existing OCP extractor."""
from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
SCRIPTS = ROOT / "fixtures/reference-histories"
SOURCE = ROOT / "fixtures/public-cad/framework/ExpansionCard_ThreadedInsert.stp"
EXPECTED = "6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09"
assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == EXPECTED
sys.path.insert(0, str(SCRIPTS))
text = (SCRIPTS / "extract_framework_open_arcs.py").read_text()
assert text.count("faces[111]") == 1
assert text.count("(-RADIUS_MM + dot") == 1
text = text.replace("faces[111]", "faces[65]")
text = text.replace("(-RADIUS_MM + dot", "(RADIUS_MM + dot")
namespace = {"__name__": "private_negative", "__file__": str(SCRIPTS / "extract_framework_open_arcs.py")}
exec(compile(text, namespace["__file__"], "exec"), namespace)
faces = namespace["faces_from_source"](SOURCE)
pipes = []
for patch, cylinder in ((66, 56), (67, 68)):
    pipe = namespace["derive_pipe"](faces, patch, cylinder)
    pipe["sourcePlaneFace"] = 65
    pipes.append(pipe)
print(json.dumps({"sourceSha256": EXPECTED, "scope": "Private same-kernel numeric diagnostic; not source equivalence.", "pipes": pipes}, indent=2))
