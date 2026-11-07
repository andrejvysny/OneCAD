# FTC09 direct STEP trace

`trace.py` reads only the pinned ASCII source, using Python's standard library.
It rejects a source hash mismatch, oversized input, malformed records, missing
references, unexpected reachable entity counts, and unsupported entities in the
traced solid. `trace.json` is a reproducible observation of that source, not a
replacement oracle.

The [source manifest](../../public-cad/manifest.json) and
[source notices](../../public-cad/README.md) retain the upstream URL, byte size,
checksum and NIST attribution for `nist_ftc_09_asme1_rd.stp`.

From the representation context and solid rooted at `#5384`, the trace records:

- Exact source decimal coordinates, vertex references and edge orientations for
  the bottom, top and two blind-pocket planar face loops.
- All 29 two-edge circular loops on the bottom face, including both circle
  placements and radii. The checker requires equal centers, radii and axis
  directions, opposite reference directions, reversed shared vertex identities,
  and matching edge orientation/sense flags. It does not infer arc parameter
  intervals or prove the two edges cover a complete circle.
- The eight source cone surface definitions and their placements, and the source
  inch, degree and closure-uncertainty declarations. Decimal values are emitted
  as strings to retain their stated precision. The supplied degree conversion
  is recorded; it is not replaced by an assumed exact value of pi/180.

The source face levels yield thickness 0.1196 in (3.03784 mm) and pocket depth
0.0500 in (1.27 mm). The reported closure uncertainty is an asserted connectivity
quantity, not a manufacturing tolerance or an error bound on mass properties.

This trace does not prove edge-curve agreement, arc trim or face orientation
semantics, exact loop closure, containment, or absence of self-intersection. It
performs no mass-property integration, interval certification, independent
kernel transfer, or native-history qualification. The preregistered `1e-6`
trial tolerance in the existing FTC09 oracle remains unqualified and unchanged.
The STEP also has supplementary entities outside the reachable solid; they are
not interpreted here.

Regenerate and test from the repository root using an existing Python virtual
environment, with no third-party packages:

```sh
python fixtures/reference-oracles/nist-ftc09-direct/trace.py > /tmp/ftc09-trace.json
cmp /tmp/ftc09-trace.json fixtures/reference-oracles/nist-ftc09-direct/trace.json
python -m unittest discover -s fixtures/reference-oracles/nist-ftc09-direct -p 'test_*.py' -v
```
