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

Existing `sha256:` revisions retain the legacy package hash rule in
`onecad-library::package`; they are never interpreted as semantic identities.
Saved placements keep their frozen geometry and revision. Because the legacy
rule cannot authenticate manifest semantics, new placement, update and re-bake
from a legacy external package are refused with an explicit diagnostic.

Semantic revisions use `sha256v2:` followed by 64 lowercase hexadecimal digits.
The canonical payload is compact UTF-8 JSON produced from the validated typed
manifest after these transformations:

- The identity revision string is replaced by the empty string. All other known
  manifest fields, including identity, source, unit, parameters, domains,
  bindings, configurations, metadata and attachment frames, are included.
- Parser defaults and the legacy generator default shim are applied; attachment
  frames are orthonormalized. Unknown fields and non-finite numbers are refused.
  Empty optional fields omitted by the typed serializer remain omitted.
- Object keys are recursively sorted; array order is retained. TOML integers and
  floats retain their distinct JSON numeric types; finite floats use the pinned
  `serde_json` serializer's shortest representation. Whitespace and source key
  order do not affect the result. Blob digests accept `sha256:` or bare spelling
  and are encoded as bare lowercase hex.

The hash input is the bytes `onecad.component.semantic.v2` followed by a zero
byte, the JSON byte count as an unsigned 64-bit big-endian integer, the JSON,
and a one-byte payload marker. Marker `0` means no source document; marker `1`
is followed by the 32 raw SHA-256 bytes of the captured source document. The
source geometry digest is part of the JSON and the blob store re-hashes captured
bytes before returning them. Package-local document files are bounded to 256
MiB. Incidental files such as previews are not part of the semantic closure.

Frozen example: the curated `onecad.std.iso4762` version `1.0.1` manifest in
`crates/onecad-app/resources/library-seed` hashes to
`sha256v2:1db4fa2a81841b17491ca7bc5ac7e8e668ded38d0c657bcfd53eb24d7e23e1eb`.
The old seed bytes remain under `library-seed-legacy`; an unchanged legacy seed
is left in place and the new version is installed alongside it. Modified or
user-authored packages are never overwritten. Old deleted seeds stay deleted.

Resolve captures and validates one owned immutable closure: identity, semantic
manifest, source, attachment frames and required bytes. Preview, commit, update
and re-bake consume that closure rather than rereading mutable package paths. A
stale index is a lookup hint, never evidence that fresh bytes have the recorded
identity. Restaging uses the document carrier or one host-owned captured draft;
it does not reopen the library. The draft shares payload bytes with the document,
is limited to 32 MiB across captured manifest, source document and geometry, and is released
on replacement or explicit release. Host integration must release the lease on
commit, cancellation and host close, and account for it in the process-wide
memory budget.

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
