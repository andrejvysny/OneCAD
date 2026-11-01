---
name: component-library-backend-gotchas
description: onecad-library/onecad-app library.rs conventions - save guard, param validation, status, seed ledger semantics
metadata:
  type: project
---

- `Library::ensure_free` refuses id@version if indexed OR `root/<id>@<version>` exists on disk; `LibraryError::AlreadyExists` maps to ApiError::InvalidCommand.
- Seed pass (`library_seed.rs`) never rewrites an existing package; editing a seed manifest only reaches fresh/deleted roots. `key` on ParameterSpec is read only by `variable_overrides` (document packages).
- No pub accessor lists timeline records from DocumentRuntime; `rt.projection().features` (op_type + id) + `rt.operation_record(RecordId)` is the pattern (`component_statuses_at`).
- `validate_component_params` kind-check only applies when the spec declares `value`; test fixtures with `key = "M6"` and no `value` skip it.
- Fresh `AlreadyExists`/`NotFound` etc. are matched via `matches!`; `new_component()` test helper in onecad-library sets a preview_png by default.
