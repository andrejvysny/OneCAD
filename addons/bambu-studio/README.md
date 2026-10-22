# Bambu Studio addon

This Rust guest is a separate sandboxed extension, discovered through its
manifest. Its **Send to Bambu Studio** command contributes to the host's Export
menu and command palette. It exports selected bodies when a selection exists,
otherwise the current document, then requests a handoff of that 3MF artifact to
the approved Bambu Studio application.

The manifest requests `export_3mf` and `launch_application`. The addon stays
subject to the host's package and application approvals. Bambu Studio must be
installed separately; its macOS bundle discovery hint is `BambuStudio.app`.
The host owns application resolution and launching. This guest has no process,
filesystem, network, or OCCT access.

## Use in OneCAD

1. Build with `make build` (or install an application bundle containing the addon).
2. Open **Home → Extensions**, find **Bambu Studio**, and review its permissions
   to enable it. On platforms without macOS bundle discovery, configure the
   absolute path to the Bambu Studio executable in the manager.
3. Open a model and optionally select the bodies to print.
4. Choose **Export → Send to Bambu Studio**, or find the command with
   **Cmd/Ctrl+Shift+P**.

OneCAD creates a 3MF file and opens it in Bambu Studio. The command does not slice
or start a printer. If launching fails, the export remains available and the host
reports its path. A successful handoff means the launcher accepted the request;
actual import and appearance must be checked in Bambu Studio.

## Build the addon separately

From the repository root:

```sh
rustup target add wasm32-unknown-unknown
scripts/build-addons.sh addons/bambu-studio
cargo test -p onecad-addon-bambu-studio
```

The generated package is staged in
`packaging/binaries/addons/dev.onecad.bambu-studio/`. Source code and the manifest
are retained in Git; the compiled component is generated during development and
packaging. See the [SDK documentation](../../crates/onecad-extension-sdk/README.md)
for the ABI, permissions, and build pipeline.
