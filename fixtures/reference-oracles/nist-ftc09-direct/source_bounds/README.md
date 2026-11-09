# FTC09 circular-pair source certificate

This source-only diagnostic reads the pinned FTC09 ASCII STEP and the existing direct trace parser. It verifies the source SHA-256 and uses exact rational arithmetic. No CAD kernel, imported B-Rep, or frozen oracle is modified.

Run from the repository root in an active Python virtual environment; no third-party packages are needed. The optional source argument must resolve inside the checkout. The report records the input and all three generator script hashes.

```sh
python fixtures/reference-oracles/nist-ftc09-direct/source_bounds/certificate.py | cmp - fixtures/reference-oracles/nist-ftc09-direct/source_bounds/certificate.json
python -m unittest discover -s fixtures/reference-oracles/nist-ftc09-direct/source_bounds -v
```

Each pair must pass explicit checks for reversed endpoint identities and coordinates, matching oriented axes and circles, antiparallel reference directions and endpoints, two forward `same_sense` flags, two forward `oriented_edge` flags, and the recorded reversed face-bound orientation. For 28 pairs, endpoints lie exactly on the circle. Subject to the stated STEP entity semantics, each directed vertex-trimmed, connected, non-self-intersecting edge occupies one of the opposite open semicircles; their loci plus shared vertices form a circle. The face-bound flag reverses use of the loop, not its locus. These are inferences, not a whole-model certification.

Loop 2794 has a nonzero exact squared-radius residual. Its circular-locus result remains conditional on endpoint reconciliation. The reported rational displacement bound describes one hypothetical radial projection; the file does not prescribe that operation, and no kernel repair or source accuracy is inferred.

The source declares a 2015 AP203 MIM schema. The publicly available [ISO 10303-42:2021 topology resource](https://ap238.org/SMRL_v8_final/data/resource_docs/geometric_and_topological_representation/sys/5_schema.htm) sections 5.5.4–5.5.6, 5.5.9 and 5.5.17 and [geometry resource](https://ap238.org/SMRL_v8_final/data/resource_docs/geometric_and_topological_representation/sys/4_schema.htm) sections 4.5.14, 4.5.19 and 4.5.26 provide the semantic basis. Applicability of that edition to the declared MIM must be independently checked before formal qualification. The checks deliberately refuse other traversal patterns, including patterns that might themselves be valid under STEP.

No global material, containment, topology, cone domain, mass-property accuracy or tolerance qualification is established. Relative angular equalities are not global angular-error bounds; source decimal exactness does not bound CAD transfer or physical measurement error.
