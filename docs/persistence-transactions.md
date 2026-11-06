# Document persistence transactions

The document host serializes explicit saves, autosaves, recovery decisions, and
close reconciliation through its persistence lane. Runtime locks protect short
snapshots and commits; they are not held while waiting for lane admission or
container I/O. A writer admitted to the lane retains ownership through its
blocking I/O even if its caller is cancelled. Close and shutdown transfer cleanup
and claims to owned tasks that finish after admitted writes drain.

Explicit save claims its target before writing. It acquires the lane before
building the payload, so a delayed save cannot overwrite a newer save with an
older snapshot. Checkpoint adoption, payload capture, and save completion are
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
atomic replacement, and parent-directory synchronization. Tests of lane ownership
and recovery fencing do not qualify the real OCCT worker, physical GPU rendering,
or device-specific lifecycle behavior; those require their respective repository
qualification gates.
