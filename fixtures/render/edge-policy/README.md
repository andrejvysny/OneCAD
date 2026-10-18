# Classified OCCT mesh fixtures

These MESH1 files contain optional edge classes and incident-face ordinals. They
are geometry fixtures, not accepted appearance goldens. Legacy fixtures elsewhere
in this directory deliberately remain unchanged for old-mesh compatibility.

Regenerate after building the pinned worker tests:

```sh
mkdir -p /private/tmp/onecad-edge-policy
ONECAD_MESH_FIXTURES=/private/tmp/onecad-edge-policy \
  worker/build-native/tests/test_tessellation_quality
```

The generating test asserts topology and classification, including the 0.001 rad
display tolerance. Review semantic changes before replacing these files. See
`worker/tests/test_tessellation_quality.cpp` and `protocol/mesh_format.md`.
