# Pre-stable format compatibility

This contract separates document, component-package and worker compatibility.
The writer remains pre-stable; native assembly qualification is tracked separately. The bounded implementation owners are ONECAD-339, ONECAD-394 and
ONECAD-520; acceptance evidence belongs in Plane.

## Documents

The current writer emits document schema 5. Explicit schema-1 through schema-4 migrations
retain immutable compatibility fixtures. The schema-1 migration places the existing part history, sketches, bodies, datum
planes and topology identities in the root definition without reauthoring them.
Existing baked `PlaceComponent` operations retain their part-timeline meaning.
The schema-2 migration preserves joint and occurrence identities. It refuses an
existing opaque `assembly.jointBindings` field rather than reinterpret it.

Opening an older file must not rewrite it. A migrated document is saved to a new
destination by default; replacing the original requires an explicit user choice.
Failed, interrupted or refused migration leaves the original bytes intact. A
reader refuses unsupported newer schema versions before adopting document state;
there is no implicit downgrade or flattening of assemblies into parts.

Schema 4 reserves `importSourceIdentity` as critical imported-face evidence in
both element references and the persisted element index. Schema-3 migration
refuses an existing key with that name rather than reinterpret opaque data.
Missing certificates retain legacy behavior. A present null, malformed or
unsupported certificate refuses admission. Certificate-bearing worker requests
require the current worker's `importSourceIdentityV1` capability and matching
kernel fingerprint. STEP entity labels and explicit XBF labels are source
addresses; replay TopoKey ordinals are not source identity. BREP source
addresses and nested occurrence paths are not admitted by this version.
End-to-end certified replay still requires worker-side production and
fail-closed enforcement; schema support alone does not qualify that workflow.

Schema 5 reserves `topologyBindingState` on element-index entries. Its only
present value is `"quarantined"`; null and unknown values are refused. Schema-4
migration refuses any pre-existing key with this name. A quarantined native
face retains its ID, body, kind, anchor and descriptor but cannot bind again.
This slice admits the marker only in embedded definitions; root-definition
markers refuse decode, edits and save until their separate runtime paths qualify.
Repair chooses a current identity explicitly. Reserved rows remain after their
last binding is repaired and count against the existing 4,096-identity cap.
They are not evicted or reused. Worker deltas cannot revive them; replay starts
from zero rather than accepting a checkpoint containing old bindings.

Automatic reservation currently requires an actual, complete worker
`NeedsRepair` answer and a closed consumer proof for the whole batch. The
implemented proof admits native Sketch/Extrude/Hole histories and Revolve
with a sketch-line axis, known local sketch geometry including arcs, and
ordinary world or face attachments. Retained synthetic reservations in the
public Prusa cover/lever histories exercise this admission and replay path;
that synthetic coverage alone does not prove worker-generated deletion/repair.
A separate production-worker regression deletes the native Prusa lever hinge
cut, retains actual `NeedsRepair` evidence, repairs through an explicit current
face, and checks Undo/Redo, manual save and fresh-process reopen before and after
repair. This is backend evidence, not native GUI or physical mechanism evidence. Validated
imports may coexist in other definitions or the unreserved root; they remain
source geometry, not native feature history. Any import in a reservation-owning
definition, even suppressed, refuses reservation. Import blobs and source
certificates retain their existing integrity checks. Unknown operations,
module state, extension metadata, unresolved feature repairs, projections and datum attachments refuse
reservation rather than being discarded or assumed unrelated. Other active
feature, selection or colour consumers also refuse it. An affected joint must
already retain the exact old reference as `needsRepair`; its accepted pose is
unchanged until an explicit repair solve succeeds. This conservative subset
does not establish general reference-repair qualification.

Reservation and its geometry publish through the existing runtime/session,
revision, source, worker-epoch and head fences, with an additional persisted
generation and inactive-staging/gesture fence. Owned consumer snapshots are
admitted against the existing shared scratch budget before cloning. Manual
saves retain the marker and evidence. Current-schema decoding, including
migrated files before any reservation exists, refuses unsupported fields that
typed decoding would discard. Recognized canonical omissions are checked by
typed location and exact default value; arbitrary null or empty unknown fields
are not exempt. Immutable legacy fixtures qualify the supported normalizations.
Untrusted files must enter through `ContainerReader` and the migration/document
IO pipeline. Direct serde decoding of `Document` is a lower-level model codec;
it does not establish the raw-field preservation proof required for admission.

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

Schema 3 joint bindings retain an occurrence path, definition, body, anchored
face identity, surface classification and explicit local X direction. The
worker supplies the exact plane or cylinder origin and Z direction; the solver
does not invent an orientation. An altered, missing or ambiguous datum requires
explicit repair and leaves the last accepted occurrence poses in place. Fresh
sessions revalidate bindings when their definitions are replayed. A document
may contain at most 40 bindings; each binding's captured evidence is limited to
64 KiB when used by the application. Read-only documents report failed
revalidation without rewriting their contents.

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
it does not reopen the library. The draft shares geometry payload bytes with the document. Its retained geometry
and exact package ZIP together are limited to 32 MiB; source capture and optional
rebake payloads are individually limited to `(32 MiB - 4096 bytes) / 3`. A
conservative ZIP-size bound is checked before allocation so the source, ZIP,
and optional rebake payloads fit the admitted reservation. Host integration
releases the lease on commit, cancellation and host close. This bounds payloads;
manifest parsing, runtime caches and save scratch still require aggregate host
accounting and separate resource qualification.

Exact verified raw manifests, source documents and required geometry are stored
in fixed-order, fixed-time ZIPs under content-addressed `library/<sha256>.zip`
entries. The `onecad.library` module slice records the semantic package identity
and archive digest in the same history transaction as the placement. Saving uses
owned archives, never a mutable package directory. Empty archive maps preserve
the existing componentless container bytes. Unknown module slices and archives
that may belong to them are preserved conservatively. Known unreferenced
archives follow the module's reference-driven save policy.

Runtime archive loading is limited to 32 MiB in aggregate. Missing, corrupt or
over-budget archives preserve supported frozen geometry but cause source-dependent
operations, and saves that would discard unavailable archival data, to refuse
explicitly. Archived snapshots support offline source-dependent edits where the
existing operation supports them; no fallback silently adopts a different
external revision. Legacy placements without archives retain their existing
external-source availability limits.

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
