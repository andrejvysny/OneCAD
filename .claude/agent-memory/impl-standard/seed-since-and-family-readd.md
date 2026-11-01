---
name: seed-since-and-family-readd
description: How to re-add a standard-part family (nut/washer) and the SeedPackage.since no-resurrection rule
metadata:
  type: project
---

Re-adding a seed family touches: worker FastenerTables.{h,cpp} + ComponentGenerators.cpp (dispatch, known_generator_ids, generator_supported_version), Rust mirror in crates/onecad-library/src/tables/, manifest in crates/onecad-app/resources/library-seed/, `seed!(id, since: N)` + SEED_VERSION bump in library_seed.rs, SCHEMA.md §7.3 + §14, fixtures registered in the `foreach(_fx place_component_...` in worker/tests/CMakeLists.txt.

`SeedPackage.since` = SEED_VERSION the package first shipped in; a missing package is installed only if previous marker < since (None marker = fresh root). Deleted-by-user stays deleted across later bumps.

Gotchas: `cargo fmt --all --check` fails only in another agent's onecad-ui files (use `rustfmt --edition 2021 <files>`); `git show 'ebdb55be^':src-tauri/...` needs the quoted ref (zsh eats `^`); unquoted `$CF` file lists don't word-split in zsh; the `foreach` fixture block in tests/CMakeLists.txt is uncommitted WIP in this tree.
