# Pre-stable format compatibility

This contract separates document, component-package and worker compatibility.
It does not declare the assembly migration implemented or a stable format
released. The bounded implementation owners are ONECAD-339, ONECAD-394 and
ONECAD-520; acceptance evidence belongs in Plane.

## Documents

The current writer emits document schema 1. Assembly schema 2 must be introduced
only with its reader, writer, explicit migration and immutable compatibility
fixtures. Migration places the existing part history, sketches, bodies, datum
planes and topology identities in the root definition without reauthoring them.
Existing baked `PlaceComponent` operations retain their part-timeline meaning.

Opening an older file must not rewrite it. A migrated document is saved to a new
destination by default; replacing the original requires an explicit user choice.
Failed, interrupted or refused migration leaves the original bytes intact. A
reader refuses unsupported newer schema versions before adopting document state;
there is no implicit downgrade or flattening of assemblies into parts.

Unknown operations, module state and extension fields must survive unchanged
where their existing opaque contracts permit editing. If their meaning cannot
be preserved by a migration, refuse that migration with a named diagnostic.
Regenerated geometry is not an excuse to discard opaque authoritative data.

Definition IDs, occurrence IDs and topology IDs have distinct roles. A stable
occurrence path is an ordered sequence of occurrence IDs; topology references
combine that path with the definition-local stable element reference. Display
rows and triangle indices are never persistent identities. Datums belong to
their definition. Future drawing edges must retain their source definition,
occurrence path and topology provenance; no namespace may reuse display IDs.

## Component packages

Existing `sha256:` revisions use the legacy package hash rule in
`onecad-library::package`. They must not be silently reinterpreted as a hash of
semantic manifest content. The replacement algorithm needs an explicit version
tag and a domain-separated canonical byte encoding.

The semantic closure includes source or generator identity and version, units,
parameter types/defaults/domains/bindings, configurations, attachment frames,
and every referenced payload digest. The revision field itself is excluded.
Canonicalization must make object-key ordering and insignificant whitespace
irrelevant. The implementation must specify array order, numeric encoding,
optional defaults and digest spelling with frozen examples.

New publication uses the new rule only after publishers, indexers, resolvers and
seed packages agree. Existing saved placements retain their exact frozen source
and identity. An unverifiable legacy external package is refused for new mutable
placement or update; it is never silently rebound to a new semantic revision.
Any explicit upgrade preserves the original package and saved document.

Resolve captures and validates one owned immutable closure: identity, semantic
manifest, source, attachment frames and required bytes. Preview, commit and save
consume that closure rather than rereading mutable paths. A stale index is a
lookup hint, never evidence that fresh bytes have the recorded identity.

Blob publication writes an exclusively owned temporary, syncs its complete
bytes, publishes without replacing an existing digest, verifies concurrent
winners, removes its own temporary and syncs the containing directory. Newly
created directory entries are synced in their parents. Failure is reported even
if publication occurred before a later sync failure. These are filesystem sync
guarantees, not proof against physical power loss or arbitrary storage-provider
behavior; those tiers require separate qualification.

## Worker and mesh

OCW1 compatibility is independent of document schema and package revision.
`protocol/SCHEMA.md` and `protocol/mesh_format.md` remain normative. Compatible
Rust/C++ readers, writers and executable fixtures precede any protocol change.
Host/worker version, binary SHA-256 and OCCT fingerprint checks remain mandatory.
Unsupported worker or mesh versions fail before authoritative publication.

MESH1 remains checked, Z-up, right-handed and outward wound, with stable topology
and row ordinals. Assembly placement does not rewrite its local geometry bytes.
The 500,000-triangle limit and aggregate resource limits remain enforced.

## Required migration evidence

Retain immutable old documents and packages. Test identity and semantic geometry,
opaque state, future-version refusal, failed/interrupted migration and fresh
process reopen. Record source and fixture hashes. A serializer round trip alone
does not prove geometry or native workflow acceptance.
