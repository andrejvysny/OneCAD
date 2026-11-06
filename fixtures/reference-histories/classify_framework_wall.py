#!/usr/bin/env python3
"""Sample native-only and lost-source points around the Framework wall cut."""
from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any

from OCP.BRepClass3d import BRepClass3d_SolidClassifier
from OCP.TopAbs import TopAbs_IN, TopAbs_OUT
from OCP.gp import gp_Pnt

from inspect_framework_residual import native_solid, source_enclosure

DOMAINS = [
    (14.19, 15.0, 2.21, 26.8, 3.0, 4.31),
    (14.0, 15.0, 26.0, 27.5, 3.0, 4.3),
]


def classify(source: Any, before: Any, after: Any, box: tuple[float, ...]) -> dict[str, Any]:
    x0, x1, y0, y1, z0, z1 = box
    classifiers = [BRepClass3d_SolidClassifier(shape) for shape in (source, before, after)]
    old_extra = new_extra = lost = uncertain = 0
    n = 20
    for i in range(n):
        for j in range(n):
            for k in range(n):
                point = gp_Pnt(x0 + (i + 0.5) * (x1 - x0) / n,
                               y0 + (j + 0.5) * (y1 - y0) / n,
                               z0 + (k + 0.5) * (z1 - z0) / n)
                for classifier in classifiers:
                    classifier.Perform(point, 1e-9)
                states = [classifier.State() for classifier in classifiers]
                if any(state not in (TopAbs_IN, TopAbs_OUT) for state in states):
                    uncertain += 1
                    continue
                source_in, before_in, after_in = [state == TopAbs_IN for state in states]
                old_extra += before_in and not source_in
                new_extra += after_in and not source_in
                lost += source_in and before_in and not after_in
    return {"boundsMm": box, "pointsPerAxis": n, "nativeOnlyBefore": old_extra,
            "nativeOnlyAfter": new_extra, "sourceNewlyRemoved": lost, "uncertain": uncertain}


def main() -> None:
    if len(sys.argv) != 4:
        raise SystemExit("usage: classify_framework_wall.py SOURCE_STEP BEFORE_STEP AFTER_STEP")
    source = source_enclosure(Path(sys.argv[1]))
    before, after = native_solid(Path(sys.argv[2])), native_solid(Path(sys.argv[3]))
    result = {"qualification": "same-kernel finite point samples; not proof of containment",
              "domains": [classify(source, before, after, box) for box in DOMAINS]}
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
