# Document persistence transactions

The document host serializes explicit saves, autosaves, recovery decisions, and
close reconciliation through its persistence lane. Runtime locks protect short
snapshots and commits; they are not held while waiting for lane admission or
container I/O. A writer admitted to the lane retains ownership through its
blocking I/O even if its caller is cancelled. Close and shutdown transfer cleanup
and claims to owned tasks that finish after admitted writes drain.

Explicit save claims its target before writing. A claim covers the canonical
parent-directory entry and any distinct existing referent, including a final
symlink; all keys are acquired and released together. An unavailable parent is
refused before writing. It acquires the lane before building the payload, so a
delayed save cannot overwrite a newer save with an older snapshot.
Checkpoint adoption, payload capture, and save completion are
fenced by the unique runtime session, including when a file with the same document
UUID is reopened. An admitted save completes and adopts its path and claim even
when its original caller stops waiting, provided that session still owns the host.

Each runtime has a persisted generation separate from its geometry revision.
Accepted document mutations and undo/redo advance it; rejected and no-op edits do
not. A published regeneration also advances it conservatively because it can
change persisted derived data. That publication alone does not dirty a clean
opened document. A save that overlaps such a publication stays dirty until a
later save. Generation exhaustion refuses mutation or publication rather than
reusing a generation.

Opaque module slices use the same transaction boundary: accepted
`set_module_state`/`SetModuleState` writes or removals, including Undo/Redo,
advance the persisted generation without interpreting the module's schema or
payload. Explicit writes may advance it conservatively even for equal payloads.
Save capture clones the complete authoritative document, including unknown
operations, module slices and extension fields allowed by its compatibility
policy. Untouched opaque content is carried forward; saving it does not create
a separate mutation. Unsupported migrations refuse rather than dropping or
reinterpreting that content. No direct mutable module/document reference is
exposed by `DocumentRuntime` to bypass these transactions.

A recovery marker may contain the producing runtime session and persisted
generation. A successful save removes a recovery pair only when its marker
positively identifies the same document and session, and its generation is no
newer than the saved payload. Missing, malformed, foreign, or newer evidence is
preserved. Explicit user discard is a separate operation. Recovery acceptance
claims the document and serializes with rejection and writers, then validates
and adopts the loaded pair before publishing a runtime. Failure to restamp the
marker preserves the offer and leaves the old runtime in place.

The low-level `autosave::write_save_payload` helper has no session or generation
proof and therefore preserves recovery files even when supplied a recovery root.
The document save API carries the proof and performs supersession checks. Marker
fields are optional for compatibility; older recovery pairs remain recoverable.

Container writes use exclusive sibling temporary files, file synchronization,
atomic replacement, and parent-directory synchronization on Unix. If that final
synchronization fails after rename, the target is already published but its
durability is unconfirmed. The save returns an error, retains recovery, and does
not adopt a new path or mark the document clean. Other platforms do not perform
this directory synchronization. Tests of lane ownership
and recovery fencing do not qualify the real OCCT worker, physical GPU rendering,
or device-specific lifecycle behavior; those require their respective repository
qualification gates.

Save admission uses the reader's inclusive limits: 64 MiB each for the document
and manifest, 256 MiB for operation history, 4 GiB total uncompressed bytes and
10,000 entries. Existing section and blob limits also apply. The manifest and
every emitted section count even when no component archives are present.
Checked arithmetic, bounded serialization and normal authoritative-document
decoding run before replacement. Prepublication refusal preserves the previous
target; it is distinct from the post-rename durability error described above.

Embedded component archives are admitted to the native host budget before their
fixed-size storage is allocated. The same retained allocation is shared by the
captured draft, runtime, and detached save payload; closing a document does not
release its charge while an admitted save still owns it. Save payloads reserve
archive decode scratch before leaving the runtime, so they can finish after the
host closes. ZIP archive sections borrow the retained bytes instead of cloning
them. Admission refusal leaves the previous runtime or save target intact.
Headless hosts without an admission callback retain the existing archive limits.

Archive pruning requires complete reference knowledge across the root and every
embedded definition. Opaque operations, modules or supported extension fields
can own references that this build cannot interpret. In that case both carried
archives and snapshot metadata are retained. Suppression and rollback do not
make a reference permanently unused.

Generator instances retain an override map separately from their complete
geometry inputs. Verified package defaults plus validated overrides determine
those inputs. Reopening a legacy mismatch backed by a carried verified snapshot
corrects root and embedded placements in one undoable transaction, invalidates
affected geometry caches and marks the document dirty. Reopen does not replace
the saved file. Undo restores the repair requirement; unverified inputs cannot
publish worker fallback geometry. Lifecycle notices remain available when scene
capture fails.

Component package publication synchronizes final payloads, the stamped manifest,
created directories and library-root ancestry before promotion, then synchronizes
the package's parent after rename. `PublishedDurabilityUnconfirmed` identifies an
already published package whose final synchronization failed. That package is
retained for reindexing; it must not be deleted as rollback. Fault-injection tests
verify ordering and reported state, not physical power-loss survival.

Catalog source resolution reserves a conservative `(32 MiB - 4096) / 3` source
allowance before reading a bounded manifest, document, and geometry closure.
Upgrade batches retain each target allowance through application. Shared source
bytes carry their allowance into the runtime and detached saves; they release it
with the last owner. This conservative allowance can refuse a batch even when its
actual source bytes would fit. File reads use fixed-size buffers and reject files
that grow during the read. Re-baked geometry reserves its file size before the
fixed-size read and retains that admission with the shared source bytes.

Archive scratch reservations are size-derived bounds for ZIP payload work. They
do not establish a bound on all TOML/JSON parser allocations, library catalog
reads, worker memory, or other model memory. Whole-process and device memory
qualification remains open.
