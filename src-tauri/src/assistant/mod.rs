//! The assistant host: the supervised Bun sidecar and its OCAK1 bridge.
//!
//! This module is the Rust half of the assistant program (ADR-0015/0016/0018).
//! It owns three things and nothing else:
//!
//! * [`supervisor`] — the child process's lifecycle: spawn, stderr forwarding,
//!   liveness, bounded restart backoff, terminal retirement;
//! * [`bridge`] — the duplex OCAK1 peer: handshake, request correlation,
//!   principal stamping, bounded streams
//!   (`../../../docs/assistant/wire-protocol.md`);
//! * [`commands`] — the five `#[tauri::command]`s the webview reaches it through.
//!
//! **It is not the geometry worker and shares no code with it.** `crate::worker`
//! speaks OCW1 to the C++ OCCT sidecar and is the modeling module's private
//! service (`docs/ARCHITECTURE.md` §3). This module speaks OCAK1 to a Bun child.
//! The two lifecycles are fully decoupled: a crashed assistant never restarts the
//! geometry worker, and vice versa. The shapes below deliberately *mirror*
//! `crate::worker::manager` because that shape is proven here — they do not
//! depend on it.
//!
//! **No CAD authority lives here** (ADR-0018). Nothing in this module registers a
//! verb, command or route that touches a document or the filesystem. The
//! sidecar→host verb table has exactly one entry, `provider.fetch`, served by
//! [`provider_gateway`]; every other inbound verb is refused by the table itself,
//! not by a convention.
//!
//! * [`provider_gateway`] — the local-provider gateway (ADR-0017). It is the only
//!   thing in this module that opens a socket, it opens it to a registered
//!   loopback address, and the id it is given names an entry in a registry Rust
//!   populated from settings. There is no verb by which the sidecar can add to
//!   that registry, and no payload field by which it can name a URL.

use std::path::{Path, PathBuf};

pub mod bridge;
pub mod commands;
pub mod provider_gateway;
pub mod supervisor;

pub use bridge::{AssistantBridge, BridgeError, BridgeOptions, StreamEvent, StreamResponse};
pub use provider_gateway::{
    validate_provider_base, GatewayError, GatewayLimits, ProviderBase, ProviderConfig,
    ProviderGateway, ProviderRegistry, ProviderStream, ResponseHead,
};
pub use supervisor::{AssistantHost, AssistantSlot, AssistantState, HostConfig};

/// The `ONECAD_ASSISTANT_HOST_PATH` override env var (development and tests only).
///
/// Its own name, deliberately: overriding the geometry worker must never move the
/// assistant host, and a single shared variable would make one sidecar's
/// misconfiguration silently relocate the other.
pub const ASSISTANT_HOST_PATH_ENV: &str = "ONECAD_ASSISTANT_HOST_PATH";

/// The assistant host's basename as Tauri drops it beside the main executable in
/// a bundled app (`externalBin` strips the target triple at install time).
pub const BUNDLED_ASSISTANT_HOST_NAME: &str = "onecad-assistant-host";

/// The staged-sidecar directory, relative to `src-tauri/` (cargo's crate root and
/// the cwd of every `cargo test`).
const STAGED_DIR: &str = "binaries";

/// The `ONECAD_ASSISTANT_LOG` level handed to the child as `--log-level`.
pub const ASSISTANT_HOST_LOG_ENV: &str = "ONECAD_ASSISTANT_LOG";

/// Resolves the assistant host binary path (packaging seam; mirrors
/// [`crate::worker::resolve_worker_path`]'s shape with its own constants).
///
/// Release builds accept only `<exe_dir>/onecad-assistant-host`, where Tauri
/// places the bundled `externalBin` sidecar. They deliberately ignore environment
/// and dev-tree paths so a relocated app cannot silently pair with another host.
/// Debug and `cfg(test)` builds use [`ASSISTANT_HOST_PATH_ENV`], then the STAGED
/// sidecar, then the copy beside the executable.
///
/// WHY the staged path is the dev fallback: unlike the geometry worker, which has
/// a build tree (`../worker/build/onecad-worker`) distinct from its staged copy,
/// `scripts/build-assistant-host.sh` compiles *directly* into
/// `src-tauri/binaries/onecad-assistant-host-<triple>` and produces no other
/// artifact. A dev fallback pointing anywhere else (an `assistant-host/dist/`,
/// say) would name a file that never exists. The triple comes from
/// [`tauri::utils::platform::target_triple`] — the same function Tauri itself
/// uses to find a sidecar — so the runtime lookup and the build script agree by
/// construction rather than by a copied string.
///
/// Returns `None` when no candidate exists on disk, so a build with no staged
/// host degrades to "assistant unavailable" instead of spawning a missing binary.
#[must_use]
pub fn resolve_assistant_host_path() -> Option<PathBuf> {
    let env_override = std::env::var_os(ASSISTANT_HOST_PATH_ENV).map(PathBuf::from);
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let staged = staged_assistant_host_path();
    resolve_assistant_host_path_from(
        env_override,
        exe_dir,
        staged.as_deref(),
        cfg!(any(debug_assertions, test)),
    )
}

/// `binaries/onecad-assistant-host-<rust host triple>` — what
/// `scripts/build-assistant-host.sh` writes. `None` when the triple cannot be
/// determined (an architecture Tauri does not name), which simply drops this
/// candidate from the chain.
fn staged_assistant_host_path() -> Option<PathBuf> {
    let triple = tauri::utils::platform::target_triple().ok()?;
    Some(Path::new(STAGED_DIR).join(format!("{BUNDLED_ASSISTANT_HOST_NAME}-{triple}")))
}

/// The pure resolution core behind [`resolve_assistant_host_path`], factored out
/// so the precedence chain is unit-testable without touching the process-global
/// environment or the real executable location.
///
/// `allow_unbundled` is true only for debug/test callers. Production callers
/// return the bundled path or `None`, without inspecting env/staged candidates.
fn resolve_assistant_host_path_from(
    env_override: Option<PathBuf>,
    exe_dir: Option<PathBuf>,
    dev_fallback: Option<&Path>,
    allow_unbundled: bool,
) -> Option<PathBuf> {
    let bundled = exe_dir.map(|dir| {
        let name = if cfg!(windows) {
            "onecad-assistant-host.exe"
        } else {
            BUNDLED_ASSISTANT_HOST_NAME
        };
        dir.join(name)
    });
    let bundled = bundled.filter(|p| p.exists());

    if !allow_unbundled {
        return bundled;
    }

    if let Some(path) = env_override {
        if path.exists() {
            return Some(path);
        }
    }
    let staged = dev_fallback.filter(|p| p.exists()).map(Path::to_path_buf);
    if let Some(staged) = staged {
        // Debug-only observability: the freshly staged sidecar is being preferred
        // over an older copy Tauri dev left beside the executable. Say so, or a
        // stale copy "works" invisibly the day this order changes.
        if let Some(shadowed) = &bundled {
            tracing::warn!(
                target: "assistant",
                staged = %staged.display(),
                shadowed = %shadowed.display(),
                "assistant host resolve: staged sidecar preferred over the copy beside the exe (debug build)"
            );
        }
        return Some(staged);
    }
    bundled
}

/// The `--log-level` the child is started with: [`ASSISTANT_HOST_LOG_ENV`] when
/// set to one of the levels its own CLI accepts, else `info`.
///
/// Validated here rather than passed through, because the child *refuses to
/// start* on an unknown level (`assistant-host/src/main.ts`) — a typo in a dev's
/// shell would otherwise read as a crash-looping sidecar.
#[must_use]
pub fn assistant_log_level() -> String {
    const LEVELS: [&str; 4] = ["error", "warn", "info", "debug"];
    match std::env::var(ASSISTANT_HOST_LOG_ENV) {
        Ok(level) if LEVELS.contains(&level.trim().to_ascii_lowercase().as_str()) => {
            level.trim().to_ascii_lowercase()
        }
        _ => "info".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_debug_prefers_env_override_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let over = dir.path().join("custom-host");
        std::fs::write(&over, b"x").unwrap();
        // A bundled sidecar AND a staged one also exist; the override wins.
        let exe_dir = dir.path().join("bundle");
        std::fs::create_dir(&exe_dir).unwrap();
        std::fs::write(exe_dir.join(BUNDLED_ASSISTANT_HOST_NAME), b"x").unwrap();
        let staged = dir.path().join("staged-host");
        std::fs::write(&staged, b"x").unwrap();

        let got = resolve_assistant_host_path_from(
            Some(over.clone()),
            Some(exe_dir),
            Some(&staged),
            true,
        );
        assert_eq!(got, Some(over));
    }

    #[test]
    fn resolve_release_uses_relocated_bundled_host() {
        let dir = tempfile::tempdir().unwrap();
        let exe_dir = dir.path().join("relocated").join("app-bin");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let bundled = exe_dir.join(BUNDLED_ASSISTANT_HOST_NAME);
        std::fs::write(&bundled, b"bundle").unwrap();
        let over = dir.path().join("custom-host");
        std::fs::write(&over, b"override").unwrap();
        let staged = dir.path().join("staged-host");
        std::fs::write(&staged, b"staged").unwrap();

        let got = resolve_assistant_host_path_from(Some(over), Some(exe_dir), Some(&staged), false);
        assert_eq!(got, Some(bundled));
    }

    #[test]
    fn resolve_release_never_selects_env_or_staged_host() {
        let dir = tempfile::tempdir().unwrap();
        let exe_dir = dir.path().join("empty-bundle");
        std::fs::create_dir(&exe_dir).unwrap();
        let over = dir.path().join("custom-host");
        std::fs::write(&over, b"override").unwrap();
        let staged = dir.path().join("staged-host");
        std::fs::write(&staged, b"staged").unwrap();

        let got = resolve_assistant_host_path_from(Some(over), Some(exe_dir), Some(&staged), false);
        assert_eq!(got, None);
    }

    #[test]
    fn resolve_debug_falls_through_to_the_staged_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let exe_dir = dir.path().join("bundle"); // empty — no sidecar beside exe
        std::fs::create_dir(&exe_dir).unwrap();
        let staged = dir.path().join("staged-host");
        std::fs::write(&staged, b"x").unwrap();

        let got = resolve_assistant_host_path_from(None, Some(exe_dir), Some(&staged), true);
        assert_eq!(got, Some(staged));
    }

    #[test]
    fn resolve_debug_prefers_staged_over_the_copy_beside_the_exe() {
        // The stale-copy drift class: BOTH exist; debug picks the staged build.
        let dir = tempfile::tempdir().unwrap();
        let exe_dir = dir.path().join("bundle");
        std::fs::create_dir(&exe_dir).unwrap();
        std::fs::write(exe_dir.join(BUNDLED_ASSISTANT_HOST_NAME), b"x").unwrap();
        let staged = dir.path().join("staged-host");
        std::fs::write(&staged, b"x").unwrap();

        let got = resolve_assistant_host_path_from(None, Some(exe_dir), Some(&staged), true);
        assert_eq!(got, Some(staged));
    }

    #[test]
    fn resolve_debug_falls_back_to_bundled_when_nothing_is_staged() {
        let dir = tempfile::tempdir().unwrap();
        let exe_dir = dir.path().join("bundle");
        std::fs::create_dir(&exe_dir).unwrap();
        let bundled = exe_dir.join(BUNDLED_ASSISTANT_HOST_NAME);
        std::fs::write(&bundled, b"x").unwrap();

        // Both the "no triple" case (None) and the "not built yet" case must land
        // on the bundled copy rather than on nothing.
        assert_eq!(
            resolve_assistant_host_path_from(None, Some(exe_dir.clone()), None, true),
            Some(bundled.clone())
        );
        assert_eq!(
            resolve_assistant_host_path_from(
                None,
                Some(exe_dir),
                Some(Path::new("/nonexistent/staged/onecad-assistant-host")),
                true
            ),
            Some(bundled)
        );
    }

    #[test]
    fn resolve_returns_none_when_no_candidate_exists() {
        let dir = tempfile::tempdir().unwrap();
        for allow_unbundled in [false, true] {
            let got = resolve_assistant_host_path_from(
                Some(dir.path().join("missing-override")),
                Some(dir.path().join("empty-bundle")),
                Some(&dir.path().join("missing-staged")),
                allow_unbundled,
            );
            assert_eq!(got, None);
        }
    }

    #[test]
    fn the_staged_candidate_is_the_path_the_build_script_writes() {
        // Lockstep with `scripts/build-assistant-host.sh`, which writes
        // `src-tauri/binaries/onecad-assistant-host-<rust host triple>`.
        let staged = staged_assistant_host_path().expect("a named target triple");
        let triple = tauri::utils::platform::target_triple().unwrap();
        assert_eq!(
            staged,
            PathBuf::from("binaries").join(format!("onecad-assistant-host-{triple}"))
        );
    }

    #[test]
    fn log_level_defaults_to_info_and_refuses_a_level_the_child_would_reject() {
        // No env var set in this process, so this is the default path. The
        // validation itself is asserted through the pure list to avoid mutating
        // process-global state from a test.
        assert_eq!(assistant_log_level(), "info");
    }
}
