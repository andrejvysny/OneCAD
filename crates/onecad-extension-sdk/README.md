# Rust extension SDK

The Rust SDK builds portable OneCAD addons as WebAssembly components. It exposes
versioned data contracts and a command export macro. It has no dependency on the
native UI, document implementation, worker transport, or extension registry.
The ABI is defined in `../onecad-extension-api/wit/extension.wit`.

An addon implements `Addon::execute` and calls `export_addon!(MyAddon)` once at
its crate root. The crate needs `crate-type = ["cdylib", "rlib"]`: the first
produces the guest, and the second supports native unit tests.

```rust,ignore
use onecad_extension_sdk::{Addon, CommandContext, CommandExecution};

struct MyAddon;
impl Addon for MyAddon {
    fn execute(command: &str, context: &CommandContext)
        -> Result<CommandExecution, String>
    {
        // Match your namespaced command and return a complete typed request plan.
        Ok(CommandExecution::default())
    }
}
onecad_extension_sdk::export_addon!(MyAddon);
```

Use [`addons/bambu-studio`](../../addons/bambu-studio) as the complete example,
including its declarative manifest and selection/document behavior. New commands
must use ids under their package's namespace. Contributions declare host-owned
locations; guests never provide widgets or call the native UI directly.

## Build and test

From the repository root, with the repository's Rust toolchain active:

```sh
rustup target add wasm32-unknown-unknown
scripts/build-addons.sh
cargo test -p onecad-addon-bambu-studio
```

The build script compiles every source package under `addons/*` that contains a
`Cargo.toml` and `manifest.json`. Pass an addon directory to build just that
package. It reads Cargo's artifact messages rather than assuming a target output
path, so `CARGO_TARGET_DIR` works. The SDK's native `build-tools` feature wraps
the core module with the canonical component ABI and validates the resulting
component. Native build dependencies are excluded from guest builds.

Staged packages contain `manifest.json` and `extension.wasm` under
`packaging/binaries/addons/<package-id>/`. This generated tree is ignored by Git.
Set `ONECAD_ADDON_STAGING_DIR` to stage elsewhere. Release packaging must run the
source build and include these staged package directories; a tracked binary or a
previous developer build is not a prerequisite.

## Execution and permissions

The component exports `execute(command, context-json) -> result<string, string>`.
The JSON strings encode `CommandContext` and `CommandExecution` from the versioned
API crate. The SDK handles serialization; addon code uses the Rust types.
Context only reports document and selection availability. It contains no paths,
process handles, authentication, worker connection, or document internals.

The guest computes a complete plan. The host validates that plan against the
manifest, package-specific approval, declared application, current context, and
runtime limits before application services perform effects. Declaring a
capability requests it; the declaration itself does not grant permission.
A package or component change requires renewed host trust review.

Version 1 supports `Export3mf` and `LaunchApplication`. The latter refers to the
opaque `PreviousExport`, so guest code cannot choose an arbitrary file to open.
Application declarations contain a name and an optional macOS bundle discovery
hint, never a shell command, executable path, or arguments. The user approves an
application through the native host. The host owns file lifetime and application
handoff. Unknown commands and invalid contexts should return errors.

Guests are built for `wasm32-unknown-unknown` and componentized without a WASI
adapter. The host supplies no filesystem, network, process, environment, or WASI
imports. Adding such imports makes a component incompatible with this runtime.
The native `rlib` target exists for tests; it is not a native plugin loading path.
There is no promise of live document callbacks, arbitrary native code loading,
or toolkit access in this ABI. Add capabilities through deliberate contract
changes rather than depending on host implementation crates.
