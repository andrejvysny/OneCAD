# Native Framework negative-retention diagnostic

Run `framework_negative_probe INPUT.onecad NEW_OUTPUT_DIR [--factor-cutter]`
through the shared
application services and the pinned production worker. The command is a diagnostic
and always exits with failure; a late or partial publication cannot qualify the
strict 30-second cold-open check. Output must be a new directory. The input is
never saved over. Runtime journals are local observations, not acceptance records.

The input is the previously captured 204-record native positive-side candidate,
SHA-256 `42ed28779aab0a220288ca27c7876c759665fc4659068cb6a2b367774c255a5a`.
The diagnostic checks its records and refuses imported blobs. New geometry uses
native sketches, extrusions, revolutions, sweeps, transforms and Booleans. An
ordinary explicit Save after cold publication and each accepted edit exercises
the existing in-session checkpoint policy; private checkpoint injection is absent.

The source-derived construction is adapted from the isolated experiment at Git
commit `7a5f90866432fb4b26794ee59ab8cef686ef2d3e`. It uses Framework Computer Inc.'s Expansion Card STEP,
SHA-256 `6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09`,
licensed under CC BY 4.0. The original notices, license and upstream reference
remain in `fixtures/public-cad/framework`. The analytic parameters and pipe
observations are derived from that source; modifications are this diagnostic's
retention dispatch and preservation of every reported split child.

For the problematic C69 retention, `P` is the copied native part intersected with
its slab, `S` is the sharp removal domain, and `D = B minus S`. Since `P` lies in
`B`, retaining `(P intersect S) minus C69` preserves the intended union with `D`.
The sphere retention distributes `B intersect (box minus sphere)` into
`(B intersect box) minus sphere`. The sphere retainer is also restricted to `S` before subtraction, using the
same identity only after union with `D`. Captured B/S/sphere/P operands precede
that restriction and must not be described as the later Cut target.

These are ideal set identities, not proofs of
OCCT boundary equivalence, source agreement or final union validity. Existing
publication checks remain authoritative. Tiny children are recorded and retained.

The default negative-side retention experiment still omits one protector
explicitly. The initial positive-side candidate also omits a protector.
Full source equivalence, protector coverage, cold readiness, Undo/Redo and fresh
reopen of the final candidate, native GUI authoring, transfer uncertainty and
ratified tolerances remain open. No production fallback, tolerance increase,
body-size deletion, imported substitute or frozen-oracle replacement is supplied.


## Factorized cutter mode

`--factor-cutter` constructs the negative sharp domain `S`, two declared
source-derived Sweep pipes and seven box-minus-primitive protectors. It first
applies one ordinary group Cut `S minus union(protectors)`, then cuts `B` with
all published cutter children. No retention body is cut against the approximate
native part before that final operation. The set identity does not establish
OCCT boundary equivalence or source coverage.

`canonical-protectors.json` journals the nine distinct declared input IDs and
masses. It is not independent proof of source completeness. If the first Cut
publishes, `published-cutter-bodies.json` records every returned child; a final
success also records checked MESH1 hashes, mass and a fresh native-only save.
A normal refusal stops authoring. This mode does not omit an unused tool or
skip a no-effect or failed operation.

The negative nine-input construction does not repair the separate positive-side
omission in the initial 204-record candidate. Strict cold readiness remains a
failure when late publication is needed, and the program still exits with
failure even if diagnostic authoring finishes. Native source equivalence and
the end-to-end reference qualification remain open.


Before the factorized group Cut, `factor-inputs/manifest.json` records accepted
B, S and nine protector BREP exports in authored order. Each export uses the
same checked preview context and snapshot; the context is rechecked before each
export and after the batch. The manifest records codec, format, solid count,
byte length, SHA-256, session/revision/worker/base-hash fences and file provenance.
Hashing uses a fixed 64 KiB buffer. File hashes attest selected file bytes; they
are not independent attestation of a running process or source equivalence.
