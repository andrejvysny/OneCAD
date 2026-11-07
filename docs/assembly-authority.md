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

The root `DocumentSession` is the single authoritative mutation and undo owner.
Flat occurrence edits and embedded definition edits enter that same history.
Repeated occurrences refer to one definition; pose, visibility, and grounding
belong to each occurrence. Make-independent copies the definition without
rewriting its local identities. Nested occurrence authoring and root-recursive
instances are refused. The assembly model validates rigid transforms, existing
definition and occurrence references, joint structure, and the existing
occurrence limits.

The default part-only open path refuses nonempty assemblies. An explicit
assembly-aware backend path exposes edits, projection, and on-demand replay;
the native UI has not yet integrated that path. Replay uses a single
process-wide auxiliary permit and a checked, transient worker session. It
fences commit by runtime session, revision, component inputs, and worker epoch;
the existing executor validates the base hash. Checked meshes remain local to
the definition and are shared by occurrences. The worker retires and is reaped
before its permit becomes available; failures retain the prior accepted scene.

The scoped element query rehydrates a definition and accepts persisted element
IDs only. Replay samples an interior triangle point for each unbound face, promotes
it on the live worker, and verifies the bindings on a fresh worker before publishing.
Unresolvable or ambiguous evidence refuses publication and retains the prior scene.
New persisted identities dirty writable documents and are limited to 4096 entries;
read-only documents refuse new identity promotion. Edge TopoKeys remain transient
and cannot be queried after worker retirement. Native pick integration is open.
Root geometry publication evicts embedded mesh caches before accepting a new scene;
components must replay again before display. Assembly recovery uses an explicit
assembly-aware API; the default part recovery continues to refuse assemblies.
Concatenating histories into one worker namespace is unsafe:
independent copies legitimately retain equal local IDs, and opaque operation
references cannot be rewritten safely. No 10-definition/20-occurrence worker
memory or latency qualification is claimed by the pure-model round trip.

`timeline/ops.jsonl` and its manifest hash continue to describe the root timeline;
`document.json` is authoritative for all definitions. Referenced import blobs
are retained from every embedded definition. Import Project refuses an assembly
rather than flattening it to a root part. MESH1 and OCW1 remain unchanged.
