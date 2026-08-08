# Kernelbench regression cases

This directory contains reviewed, replayable KBR-0 cases. Each case must
validate against `../schemas/case-v1.schema.json`, use generator version 1, and
retain its original 16-digit lowercase hexadecimal seed.

Add a case only after reproducing it in a fresh runner process. Prefer the
smallest semantic recipe and selector that preserve the behavior. Selectors
must use matching generator provenance, topology roles, recipe-local anchors,
surface descriptors, and adjacency. Never save raw OCCT edge or face ordinals
from a generated BREP.

Regression files are benchmark inputs, not production tolerance policy and not
members of the frozen legacy `corpus/`. Store failure artifacts in run output
directories, not here.
