"""Input and output guards for the pinned Robonine conversion tools."""

import hashlib
from pathlib import Path

import OCP


SOURCE_SHA256 = {
    "clamp_1": "a9540533009f872423431d8405613ee52a609e2d296a287c17867f941aae50ca",
    "clamp_2": "0a1c2caf47b97853e4dc0974efc5ed89f9969b89cae3c0b8f8d9e8705bff3498",
}


def check_version() -> None:
    if OCP.__version__ != "8.0.1.1":
        raise RuntimeError(f"OCP 8.0.1.1 required, found {OCP.__version__}")


def check_source(name: str, path: Path) -> None:
    expected = SOURCE_SHA256.get(name)
    if expected is None:
        raise ValueError(f"unknown pinned source: {name}")
    if path.stat().st_size != 440484:
        raise ValueError(f"source size mismatch: {path}")
    with path.open("rb") as stream:
        data = stream.read(440485)
    if len(data) != 440484 or hashlib.sha256(data).hexdigest() != expected:
        raise ValueError(f"source size or SHA-256 mismatch: {path}")


def require_new(path: Path) -> None:
    if path.exists():
        raise FileExistsError(f"output already exists: {path}")
