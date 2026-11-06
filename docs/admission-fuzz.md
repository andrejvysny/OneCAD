# Bounded admission mutation campaigns

Run from the repository root:

```sh
cargo test -p onecad-core -p onecad-protocol --test bounded_admission_fuzz --test bounded_mesh_fuzz -- --nocapture
```

These deterministic mutation campaigns exercise the production document parser,
container reader, and checked MESH1 decoder. They are CPU admission checks, not
worker, GPU, native-input, migration-interruption, or physical-storage qualification.
They do not replace the required worker and native acceptance lanes.

The document/container campaign uses seed `4220339039400002`: schema-2 embedded
definitions, opaque module state, a future document schema, an invalid rigid
transform, a current container, and the immutable schema-1 plate fixture. It runs
1,536 document and 512 container mutations. Container seeds are limited to 4 MiB;
admission must not modify either the candidate or the saved original.

The checked-mesh campaign uses seed `42200c014d455348`, with 8,192 mutations of
the frozen worker-produced box and cylinder MESH1 fixtures. Successful decodes
must remain safe to traverse. Production decoder limits remain unchanged.

The scheduled CI job has a 15-minute limit and retains source, toolchain, OS,
case counts, elapsed times, and command output even on failure. Fixed mutations
are reproducible coverage, not a coverage-guided fuzzing completeness claim.
Public native-history containers must be added when independently qualified;
the currently admitted public STEP files are not document-container seeds.
Preserve any discovered failure as a minimized deterministic regression before
expanding the campaign. Do not regenerate frozen inputs to hide a failure.
