# Assembly authority and migration

Schema 2 stores the former part fields once in `rootDefinition`; document identity,
settings, module state, and document-level extensions remain document-owned.
Embedded definitions contain their own timeline, sketches, datums, bodies,
elements, variables, repair state, and definition extensions. The root fields
remain available through the existing in-memory `Document` API; they do not
constitute a second serialized copy.

The schema-1 reader moves the old part fields into the root definition without
reauthoring their IDs or reinterpreting `PlaceComponent`. It derives the root
identity from the document identity using domain-separated SHA-256. A schema-1
extension that collides with a reserved schema-2 assembly key is refused, rather
than discarded. The original file is not rewritten on open. Migrated documents
have no default save target, so replacement of the source requires an explicit
Save As selection. Future document schemas are refused. Unknown document,
module, definition, and operation fields retain their existing opaque contracts.

The native Assembly inspector inserts repeated definitions, places/hides/grounds
individual occurrences, and makes an independent copy of a selected occurrence.
Its bounded shared-geometry edit adds a local translation feature to every body
in the selected definition; all its instances update together. The command
requires current accepted definition geometry and follows the same generation
fence and document Undo/Redo path. General feature/sketch editing inside an
embedded definition remains a separate native authoring gap; the bounded
translation control does not establish full part-editor parity.

The root `DocumentSession` is the single authoritative mutation and undo owner.
Flat occurrence edits and embedded definition edits enter that same history.
Repeated occurrences refer to one definition; pose, visibility, and grounding
belong to each occurrence. Make-independent copies the definition without
rewriting its local identities. Nested occurrence authoring and root-recursive
instances are refused. The assembly model validates rigid transforms, existing
definition and occurrence references, joint structure, and the existing
occurrence limits.

The default part-only open path refuses nonempty assemblies. An explicit
assembly-aware backend path exposes edits, projection, and on-demand replay.
The native host uses this path for open and recovery, captures shared local
geometry with occurrence-qualified selection authority, and refuses child
selection as input to root-only modeling operations. Replay uses a single
process-wide auxiliary permit and a checked, transient worker session. It
fences commit by runtime session, revision, component inputs, and worker epoch;
the existing executor validates the base hash. Checked meshes remain local to
the definition and are shared by occurrences. The worker retires and is reaped
before its permit becomes available; failures retain the prior accepted scene.

The scoped element query rehydrates a definition and accepts persisted element
IDs only. Replay samples an interior triangle point for each unbound face, promotes
it on the live worker, and verifies the bindings on a fresh worker before publishing.
Unresolvable or ambiguous evidence refuses publication and retains the prior scene.
Any changed persisted definition state dirties writable documents. Persisted
identities are limited to 4096 entries;
read-only documents refuse new identity promotion. Edge TopoKeys remain transient
and cannot be queried after worker retirement. Native selection retains source
body and occurrence path separately from its collision-checked display identity.
Occurrence section views and root modeling operands are refused explicitly.
Root geometry publication evicts embedded mesh caches before accepting a new scene;
components must replay again before display. Assembly recovery uses an explicit
assembly-aware API; the default part recovery continues to refuse assemblies.
Concatenating histories into one worker namespace is unsafe:
independent copies legitimately retain equal local IDs, and opaque operation
references cannot be rewritten safely. The real-worker regression covers 10 definitions and 20 occurrences through
fresh reopen. This does not establish memory, latency, physical input, or latest
public stable OS qualification.

`timeline/ops.jsonl` and its manifest hash continue to describe the root timeline;
`document.json` is authoritative for all definitions. Referenced import blobs
are retained from every embedded definition. Import Project refuses an assembly rather than flattening it to a root part.
It also refuses documents carrying component-package metadata or archive entries
until that source closure can be imported atomically; opening such a document
directly preserves it. MESH1 and OCW1 remain unchanged.

For regular acyclic joint graphs with at most one grounded body per connected
component, the solver certifies local equality-constraint rank structurally.
Each tree edge introduces six relative coordinates and contributes six rigid
rows, five free slider/revolute rows, or six driven/equality-bound rows. Floating
components retain six global DOFs. The certificate requires a rotation-chart
regularity check independent of the requested solve tolerance, and preserves the
coordinate representability guard. Cycles and multiple grounds retain the
conservative two-step numerical estimate; no general rank proof is claimed.
Final residual and limit validation still precedes publication.

A zero-width limit is an equality target even when the current value already
satisfies it. A one-sided endpoint retains inward motion. Driven revolute scalar
finite differences use circular differences across the wrapped-error seam;
residuals, tolerances and iteration caps are unchanged. Captured source-grounded
solver inputs and symbolic DOF expectations live under
`crates/onecad-assembly/tests/fixtures`.
