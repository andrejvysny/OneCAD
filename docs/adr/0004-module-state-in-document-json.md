# 0004 — Module state lives in `document.json`, not new container paths

- Status: Accepted
- Date: 2026-08-08

## Context

The `.onecad` v2 container is already a ZIP with semantic sections:
`manifest.json`, authoritative `document.json`, `timeline/ops.jsonl`, geometry and
mesh caches, `imports/<sha256>` blobs, `preview.png`. It has per-entry SHA-256
integrity, atomic save, and a guarded reader with entry-count, size and traversal
limits.

A namespaced module layout could be expressed either as new archive paths
(`modules/<id>/state.json`) or as a map inside the existing authoritative
document.

## Decision

For v1, module state is a `modules` map inside `document.json`:

```json
{
  "modules": {
    "onecad.modeling": { "schemaVersion": 2, "payload": { } },
    "com.example.foo": { "schemaVersion": 1, "payload": { } }
  }
}
```

with a descriptor table in `manifest.json` recording which modules a document
uses and at which schema version, so a missing module can be reported.

Reasons:

- one authoritative writer, so atomic save and the manifest integrity table cover
  module state with no new machinery;
- no new archive-guard surface — nothing to path-traverse or zip-bomb through;
- no container version bump and therefore no user-document migration. Serializing
  the field only when non-empty keeps existing documents byte-identical.

Separate archive paths remain the documented migration for when payload sizes make
loading the whole document expensive. Large binary data should go to the resource
store, not into this map, in either design.

## Cost

A very large module payload is read and written with the document rather than
lazily. Accepted for v1; moving to archive paths later is a container-format
change, which is exactly why it is recorded here rather than left implicit.
