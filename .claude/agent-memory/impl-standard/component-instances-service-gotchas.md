---
name: component-instances-service-gotchas
description: library_instances.rs upgrade/reset service - set_component_params merges so removal needs the _removing_ variant; dropped override changes geometry in real-worker tests
metadata:
  type: project
---

- `set_component_params_at` MERGES; keys can only leave a record via `set_component_params_removing_at` (library.rs, pub(crate)); Profile sources refill the package default for removed keys.
- Upgrade apply = re-plan + equality vs approved + per-record snapshot re-check under the final runtime lock, then `apply_transaction` (one undo). Staged blobs are not rolled back on a failed transaction (same as `stage_blob_and_apply`).
- Real-worker upgrade tests: a dropped override (e.g. thread_detail simplified -> cosmetic) CHANGES the exact volume; assert expected volumes accordingly.
- Test packages: dir name `<id>@<version>`, distinct `revision` per version, reindex after writing v1.1.0.
