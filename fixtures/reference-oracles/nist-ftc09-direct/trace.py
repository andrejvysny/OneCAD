#!/usr/bin/env python3
"""Trace bounded FTC09 STEP evidence without a geometry kernel."""
from __future__ import annotations

import hashlib
import json
import re
import sys
from collections import Counter
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation
from pathlib import Path

SHA256 = "d12866d3ab0fc260efd0f1a5c54af47950be513486f61f65513877045582afa8"
MAX_BYTES = 1024 * 1024
MAX_ENTITIES = 10000
TOKEN = re.compile(r"\s*(?:(#[0-9]+)|('(?:''|[^'])*')|(\.[A-Z_]+\.)|([A-Z_][A-Z_0-9]*)|([+-]?(?:\d+\.?\d*|\.\d+)(?:E[+-]?\d+)?)|([(),*$]))")
EXPECTED_TYPES = {
    "ADVANCED_FACE": 158, "AXIS2_PLACEMENT_3D": 342, "CARTESIAN_POINT": 912,
    "CIRCLE": 184, "CLOSED_SHELL": 1, "CONICAL_SURFACE": 8,
    "CYLINDRICAL_SURFACE": 88, "DIRECTION": 954, "EDGE_CURVE": 454,
    "EDGE_LOOP": 228, "FACE_BOUND": 70, "FACE_OUTER_BOUND": 158,
    "LINE": 270, "MANIFOLD_SOLID_BREP": 1, "ORIENTED_EDGE": 908,
    "PLANE": 62, "VECTOR": 270, "VERTEX_POINT": 300,
}

@dataclass(frozen=True)
class Ref:
    id: int

@dataclass(frozen=True)
class Entity:
    name: str
    args: tuple[object, ...]


def tokens(text: str) -> list[str]:
    out = []
    while text.strip():
        match = TOKEN.match(text)
        if not match:
            raise ValueError("unsupported STEP token")
        out.append(next(value for value in match.groups() if value is not None))
        text = text[match.end():]
    if len(out) > 10000:
        raise ValueError("entity token limit")
    return out


def parse_value(items: list[str], pos: int = 0, depth: int = 0) -> tuple[object, int]:
    if depth > 32 or pos >= len(items):
        raise ValueError("invalid or deeply nested value")
    value = items[pos]
    if value == "(":
        values = []
        pos += 1
        while pos < len(items) and items[pos] != ")":
            item, pos = parse_value(items, pos, depth + 1)
            values.append(item)
            if pos < len(items) and items[pos] == ",":
                pos += 1
            elif pos >= len(items) or items[pos] != ")":
                raise ValueError("missing separator")
        if pos >= len(items):
            raise ValueError("unclosed list")
        return tuple(values), pos + 1
    if re.fullmatch(r"[A-Z_][A-Z_0-9]*", value):
        args, end = parse_value(items, pos + 1, depth + 1)
        if not isinstance(args, tuple):
            raise ValueError("entity requires arguments")
        return Entity(value, args), end
    if value.startswith("#"):
        return Ref(int(value[1:])), pos + 1
    if value.startswith("'") or value in ("*", "$") or value.startswith(".") and value.endswith("."):
        return value, pos + 1
    try:
        number = Decimal(value)
    except InvalidOperation as exc:
        raise ValueError("invalid scalar") from exc
    if not number.is_finite():
        raise ValueError("nonfinite scalar")
    return number, pos + 1


def parse_step(data: bytes, require_hash: bool = True) -> dict[int, str]:
    if len(data) > MAX_BYTES:
        raise ValueError("source byte limit")
    if require_hash and hashlib.sha256(data).hexdigest() != SHA256:
        raise ValueError("source SHA-256 mismatch")
    try:
        text = data.decode("ascii")
    except UnicodeDecodeError as exc:
        raise ValueError("non-ASCII source") from exc
    if text.count("DATA;") != 1 or text.count("ENDSEC;") != 2:
        raise ValueError("unsupported STEP sections")
    section = text.split("DATA;", 1)[1].split("ENDSEC;", 1)[0]
    records = {}
    for record in section.split(";"):
        if not record.strip():
            continue
        match = re.fullmatch(r"\s*#(\d+)\s*=\s*(.*?)\s*", record, re.S)
        if not match:
            raise ValueError("malformed entity record")
        key = int(match[1])
        if key in records:
            raise ValueError("duplicate entity id")
        records[key] = match[2]
        if len(records) > MAX_ENTITIES:
            raise ValueError("entity count limit")
    return records


def entity(records: dict[int, str], key: int, expected: str | None = None) -> Entity:
    if key not in records:
        raise ValueError(f"missing entity #{key}")
    values = tokens(records[key])
    result, end = parse_value(values)
    if end != len(values) or not isinstance(result, Entity):
        raise ValueError(f"unsupported entity #{key}")
    if expected and result.name != expected:
        raise ValueError(f"#{key}: expected {expected}, got {result.name}")
    return result


def refs(value: object) -> list[int]:
    if isinstance(value, Ref):
        return [value.id]
    if isinstance(value, Entity):
        return refs(value.args)
    if isinstance(value, tuple):
        return [item for child in value for item in refs(child)]
    return []


def graph(records: dict[int, str], root: int) -> Counter[str]:
    todo = [root]
    seen: set[int] = set()
    counts: Counter[str] = Counter()
    while todo:
        key = todo.pop()
        if key in seen:
            continue
        if len(seen) >= MAX_ENTITIES:
            raise ValueError("reachable entity limit")
        seen.add(key)
        node = entity(records, key)
        if node.name not in EXPECTED_TYPES:
            raise ValueError("unsupported reachable entity")
        counts[node.name] += 1
        todo.extend(refs(node))
    return counts


def require_ref(value: object) -> int:
    if not isinstance(value, Ref):
        raise ValueError("expected entity reference")
    return value.id


def vector(records: dict[int, str], key: int, kind: str) -> tuple[Decimal, ...]:
    args = entity(records, key, kind).args
    if len(args) != 2 or not isinstance(args[1], tuple) or len(args[1]) != 3:
        raise ValueError(f"invalid {kind}")
    if not all(isinstance(x, Decimal) for x in args[1]):
        raise ValueError(f"invalid {kind} coordinates")
    return args[1]


def placement(records: dict[int, str], key: int) -> dict[str, object]:
    args = entity(records, key, "AXIS2_PLACEMENT_3D").args
    if len(args) != 4:
        raise ValueError("invalid placement")
    return {"point": vector(records, require_ref(args[1]), "CARTESIAN_POINT"),
            "axis": vector(records, require_ref(args[2]), "DIRECTION"),
            "reference": vector(records, require_ref(args[3]), "DIRECTION")}


def vertex_point(records: dict[int, str], key: int) -> tuple[Decimal, ...]:
    vertex = entity(records, key, "VERTEX_POINT")
    if len(vertex.args) != 2:
        raise ValueError("invalid vertex")
    return vector(records, require_ref(vertex.args[1]), "CARTESIAN_POINT")


def loop_edges(records: dict[int, str], loop: int) -> list[dict[str, object]]:
    args = entity(records, loop, "EDGE_LOOP").args
    if len(args) != 2 or not isinstance(args[1], tuple) or not args[1]:
        raise ValueError("invalid edge loop")
    result = []
    for ref in args[1]:
        oe = require_ref(ref)
        oriented = entity(records, oe, "ORIENTED_EDGE").args
        if len(oriented) != 5 or oriented[1:3] != ("*", "*") or oriented[4] not in (".T.", ".F."):
            raise ValueError("unsupported oriented edge")
        edge_id = require_ref(oriented[3])
        edge = entity(records, edge_id, "EDGE_CURVE").args
        if len(edge) != 5 or edge[4] not in (".T.", ".F."):
            raise ValueError("unsupported edge curve")
        result.append({"orientedEdge": oe, "orientation": oriented[4], "edge": edge_id,
                       "vertices": [require_ref(edge[1]), require_ref(edge[2])],
                       "curve": require_ref(edge[3]), "sameSense": edge[4],
                       "vertexPointsInch": [vertex_point(records, require_ref(edge[1])),
                                            vertex_point(records, require_ref(edge[2]))]})
    return result


def face(records: dict[int, str], face_id: int) -> dict[str, object]:
    args = entity(records, face_id, "ADVANCED_FACE").args
    if len(args) != 4 or not isinstance(args[1], tuple) or args[3] not in (".T.", ".F."):
        raise ValueError("invalid face")
    plane_id = require_ref(args[2])
    plane = entity(records, plane_id, "PLANE")
    frame = placement(records, require_ref(plane.args[1]))
    bounds = []
    for ref in args[1]:
        bound_id = require_ref(ref)
        bound = entity(records, bound_id)
        if bound.name not in ("FACE_BOUND", "FACE_OUTER_BOUND") or len(bound.args) != 3 or bound.args[2] not in (".T.", ".F."):
            raise ValueError("unsupported face bound")
        loop = require_ref(bound.args[1])
        bounds.append({"id": bound_id, "kind": bound.name, "orientation": bound.args[2],
                       "loop": loop, "edges": loop_edges(records, loop)})
    return {"id": face_id, "plane": plane_id, "placement": frame,
            "sameSense": args[3], "bounds": bounds}


def circle(records: dict[int, str], key: int) -> dict[str, object]:
    args = entity(records, key, "CIRCLE").args
    if len(args) != 3 or not isinstance(args[2], Decimal) or args[2] <= 0:
        raise ValueError("invalid circle")
    return {"id": key, "placement": placement(records, require_ref(args[1])), "radius": args[2]}


def verify_pairs(records: dict[int, str], bottom: dict[str, object]) -> list[dict[str, object]]:
    result = []
    for bound in bottom["bounds"]:
        edges = bound["edges"]
        if len(edges) != 2:
            continue
        a, b = edges
        ca, cb = circle(records, a["curve"]), circle(records, b["curve"])
        if ca["radius"] != cb["radius"] or ca["placement"]["point"] != cb["placement"]["point"] or ca["placement"]["axis"] != cb["placement"]["axis"]:
            raise ValueError("circle pair geometry mismatch")
        ar, br = ca["placement"]["reference"], cb["placement"]["reference"]
        if ar != tuple(-value for value in br):
            raise ValueError("circle pair reference axes are not opposite")
        if a["vertices"] != list(reversed(b["vertices"])):
            raise ValueError("circle pair does not share reversed vertices")
        if a["orientation"] != b["orientation"] or a["sameSense"] != b["sameSense"]:
            raise ValueError("circle pair orientation mismatch")
        for vid in a["vertices"]:
            vertex = entity(records, vid, "VERTEX_POINT")
            vector(records, require_ref(vertex.args[1]), "CARTESIAN_POINT")
        result.append({"loop": bound["loop"], "boundOrientation": bound["orientation"],
                       "edges": edges, "circles": [ca, cb]})
    if len(result) != 29:
        raise ValueError("unexpected two-edge circle loop count")
    return result


def main_trace(data: bytes) -> dict[str, object]:
    records = parse_step(data)
    counts = graph(records, 5384)
    if counts != EXPECTED_TYPES:
        raise ValueError("unexpected reachable topology")
    if records[5777].replace(" ", "") != "LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(2.54E1),#5776)":
        raise ValueError("unexpected length conversion")
    if "CONVERSION_BASED_UNIT('INCH',#5777)" not in records[5778].replace(" ", ""):
        raise ValueError("unexpected inch unit")
    bottom, top, floor1, floor2 = (face(records, key) for key in (2920, 3321, 5194, 5226))
    if [len(x["bounds"]) for x in (bottom, top, floor1, floor2)] != [35, 37, 1, 1]:
        raise ValueError("unexpected face bound counts")
    if [x["placement"]["point"][1] for x in (bottom, top, floor1, floor2)] != list(map(Decimal, ("0", ".1196", ".0696", ".0696"))):
        raise ValueError("unexpected face levels")
    pairs = verify_pairs(records, bottom)
    cones = []
    for key in (2940, 3508, 3533, 3548, 3587, 3602, 3641, 3656):
        c = entity(records, key, "CONICAL_SURFACE")
        if len(c.args) != 4 or not isinstance(c.args[2], Decimal) or c.args[3] != Decimal(45):
            raise ValueError("unexpected cone parameters")
        cones.append({"id": key, "placement": placement(records, require_ref(c.args[1])),
                      "radiusInch": c.args[2], "semiangleDegrees": c.args[3]})
    factor = entity(records, 5780, "PLANE_ANGLE_MEASURE_WITH_UNIT")
    if len(factor.args) != 2 or not isinstance(factor.args[0], Entity) or factor.args[0].name != "PLANE_ANGLE_MEASURE" or factor.args[0].args != (Decimal("1.745329251994E-2"),) or factor.args[1] != Ref(5779):
        raise ValueError("unexpected angle conversion")
    if "CONVERSION_BASED_UNIT('DEGREE',#5780)" not in re.sub(r"\s+", "", records[5781]):
        raise ValueError("unexpected angle unit")
    context = re.sub(r"\s+", "", records[5784])
    required = ("GEOMETRIC_REPRESENTATION_CONTEXT(3)",
                "GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#5783))",
                "GLOBAL_UNIT_ASSIGNED_CONTEXT((#5778,#5781,#5782))")
    if any(value not in context for value in required):
        raise ValueError("unexpected representation units or uncertainty")
    if entity(records, 5785, "ADVANCED_BREP_SHAPE_REPRESENTATION").args != (
        "''", (Ref(5384),), Ref(5784)
    ):
        raise ValueError("unexpected representation context")
    if re.sub(r"\s+", "", records[5776]) != "(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.))":
        raise ValueError("unexpected millimeter unit")
    if re.sub(r"\s+", "", records[5779]) != "(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.))":
        raise ValueError("unexpected radian unit")
    return {"sourceSha256": SHA256, "scope": "raw STEP trace; not a geometry or tolerance qualification",
            "reachableEntityCounts": dict(sorted(counts.items())),
            "lengthUnit": {"id": 5778, "conversionId": 5777, "millimetersPerInch": Decimal('25.4')},
            "angleUnit": {"id": 5781, "conversionId": 5780, "sourceExpression": records[5780]},
            "connectivityUncertainty": {"id": 5783, "sourceExpression": records[5783]},
            "faces": [bottom, top, floor1, floor2], "twoEdgeCirclePairs": pairs,
            "cones": cones, "unresolved": ["No source continuity or trim-semantics proof.",
                "No interval integral, independent kernel check, or tolerance qualification."]}


def encode(value: object) -> object:
    if isinstance(value, Decimal):
        return str(value)
    raise TypeError(type(value).__name__)


def read_source(path: Path) -> bytes:
    with path.open("rb") as source:
        data = source.read(MAX_BYTES + 1)
    if len(data) > MAX_BYTES:
        raise ValueError("source byte limit")
    return data


def main() -> None:
    source = Path(__file__).resolve().parents[2] / "public-cad/nist/nist_ftc_09_asme1_rd.stp"
    result = main_trace(read_source(source))
    sys.stdout.write(json.dumps(result, default=encode, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
