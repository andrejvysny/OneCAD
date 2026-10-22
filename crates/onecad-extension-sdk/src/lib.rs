//! Toolkit-independent Rust SDK for sandboxed OneCAD extension components.
//!
//! An addon returns a plan of requests. The host validates capabilities and
//! permissions before executing that plan through application services.

pub use onecad_extension_api::{
    ApplicationContribution, ArtifactRef, Capability, CommandContext, CommandContribution,
    CommandExecution, CommandMenu, Contributions, ExportScope, ExtensionRuntime, HostRequest,
    Manifest,
};

#[doc(hidden)]
pub mod bindings {
    wit_bindgen::generate!({
        path: "../onecad-extension-api/wit",
        world: "addon",
        pub_export_macro: true,
    });
}

/// Implements a command without exposing host internals or native handles.
pub trait Addon {
    fn execute(command: &str, context: &CommandContext) -> Result<CommandExecution, String>;
}

/// Decode the versioned JSON contract at the component boundary.
#[doc(hidden)]
pub fn execute<A: Addon>(command: String, context_json: String) -> Result<String, String> {
    let context = serde_json::from_str(&context_json)
        .map_err(|error| format!("invalid command context: {error}"))?;
    let execution = A::execute(&command, &context)?;
    serde_json::to_string(&execution)
        .map_err(|error| format!("cannot encode command execution: {error}"))
}

/// Export an [`Addon`] implementation using OneCAD's component ABI.
///
/// Invoke once at the guest crate root. Build as a `cdylib` for
/// `wasm32-unknown-unknown`, then use `scripts/build-addons.sh` to componentize it.
#[macro_export]
macro_rules! export_addon {
    ($addon:ty) => {
        struct OneCadGuest;

        impl $crate::bindings::Guest for OneCadGuest {
            fn execute(command: String, context_json: String) -> Result<String, String> {
                $crate::execute::<$addon>(command, context_json)
            }
        }

        $crate::bindings::export!(OneCadGuest with_types_in $crate::bindings);
    };
}
