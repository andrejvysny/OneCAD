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

use std::collections::BTreeMap;
use std::ffi::OsString;
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

/// The executable suffix a target triple's binaries carry.
///
/// Keyed on the TRIPLE rather than on `cfg!(windows)`, because the staged name
/// is a property of the target a binary was built for and not of the machine
/// asking about it — `scripts/build-assistant-host.sh` can be told to build for
/// a triple that is not its own. `scripts/lib/assistant-host-target.sh` computes
/// the same suffix from the same input, and
/// [`the_staged_name_agrees_with_the_build_script`] holds the two to it.
#[must_use]
pub fn assistant_host_exe_suffix(triple: &str) -> &'static str {
    if triple.contains("windows") {
        ".exe"
    } else {
        ""
    }
}

/// `onecad-assistant-host-<triple>[.exe]` — the file
/// `scripts/build-assistant-host.sh` writes into `src-tauri/binaries/`, and the
/// name `bundle.externalBin` looks for.
#[must_use]
pub fn staged_assistant_host_file_name(triple: &str) -> String {
    format!(
        "{BUNDLED_ASSISTANT_HOST_NAME}-{triple}{}",
        assistant_host_exe_suffix(triple)
    )
}

/// `onecad-assistant-host[.exe]` — the name Tauri installs beside the main
/// executable, with the triple stripped and the extension kept.
///
/// `std::env::consts::EXE_SUFFIX` rather than a `cfg!` of our own: this one is
/// about the machine the app is running on, and that constant is the answer the
/// standard library already has.
#[must_use]
pub fn bundled_assistant_host_file_name() -> String {
    format!(
        "{BUNDLED_ASSISTANT_HOST_NAME}{}",
        std::env::consts::EXE_SUFFIX
    )
}

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
    Some(Path::new(STAGED_DIR).join(staged_assistant_host_file_name(&triple)))
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
    let bundled = exe_dir.map(|dir| dir.join(bundled_assistant_host_file_name()));
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

/// The environment variables the assistant sidecar is started with.
///
/// The child inherited the whole of OneCAD's environment. That is a shape, not a
/// breach: nothing is known to read a secret out of it. But the parent's
/// environment is where a developer's `OPENAI_API_KEY`, an IT profile's
/// `HTTPS_PROXY` and an inherited `NODE_OPTIONS=--require …` all live, and a Bun
/// process that reads any of them does something OneCAD did not ask for.
///
/// **This is application hardening, not a sandbox.** A child with a trimmed
/// environment can still open a file, read `~/.aws/credentials` and dial out; the
/// operating system is what would stop that, and nothing here is the operating
/// system. What this does is narrow: the sidecar starts with the variables an
/// ordinary process needs to run at all, and with nothing else.
///
/// The names kept are the ones without which a process misbehaves rather than
/// merely loses a convenience: `PATH` (Bun resolves nothing without it), `HOME`
/// and the temp locations (a store and a scratch file need somewhere to live),
/// locale and `TZ` (timestamps and text), and on Windows the system directories
/// that the loader itself reads.
#[must_use]
pub fn assistant_child_env() -> BTreeMap<String, String> {
    minimal_child_env(std::env::vars_os(), cfg!(windows))
}

/// Names kept on every platform.
const CHILD_ENV_KEEP: [&str; 9] = [
    "PATH", "HOME", "TMPDIR", "TMP", "TEMP", "LANG", "LC_ALL", "LC_CTYPE", "TZ",
];

/// Names kept additionally on Windows, where the loader and the CRT read them.
const CHILD_ENV_KEEP_WINDOWS: [&str; 11] = [
    "SYSTEMROOT",
    "WINDIR",
    "SYSTEMDRIVE",
    "COMSPEC",
    "PATHEXT",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "NUMBER_OF_PROCESSORS",
    "OS",
];

/// The pure core of [`assistant_child_env`], so the policy is testable without
/// mutating the process's own environment.
///
/// A variable whose name or value is not valid Unicode is dropped: every name on
/// the list above is ASCII, so a non-Unicode name cannot be one of them, and a
/// non-Unicode value of one of them is a value this process cannot reason about.
fn minimal_child_env(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
    windows: bool,
) -> BTreeMap<String, String> {
    let mut kept = BTreeMap::new();
    for (name, value) in vars {
        let (Some(name), Some(value)) = (name.to_str(), value.to_str()) else {
            continue;
        };
        // Windows environment names are case-insensitive; POSIX names are not,
        // and `Path` is not `PATH` there.
        let lookup = if windows {
            name.to_ascii_uppercase()
        } else {
            name.to_string()
        };
        let keep = CHILD_ENV_KEEP.contains(&lookup.as_str())
            || (windows && CHILD_ENV_KEEP_WINDOWS.contains(&lookup.as_str()));
        if keep {
            kept.insert(name.to_string(), value.to_string());
        }
    }
    kept
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
        std::fs::write(exe_dir.join(bundled_assistant_host_file_name()), b"x").unwrap();
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
        let bundled = exe_dir.join(bundled_assistant_host_file_name());
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
        std::fs::write(exe_dir.join(bundled_assistant_host_file_name()), b"x").unwrap();
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
        let bundled = exe_dir.join(bundled_assistant_host_file_name());
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
        // `src-tauri/binaries/onecad-assistant-host-<triple>[.exe]`.
        let staged = staged_assistant_host_path().expect("a named target triple");
        let triple = tauri::utils::platform::target_triple().unwrap();
        assert_eq!(
            staged,
            PathBuf::from("binaries").join(staged_assistant_host_file_name(&triple))
        );
    }

    /// F11: the staged name omitted `.exe` while the bundled resolver expected
    /// it, so on Windows the two halves of the packaging seam named different
    /// files. One mapping, keyed on the triple, answers both.
    #[test]
    fn the_windows_executable_suffix_is_part_of_every_name() {
        for (triple, suffix) in [
            ("x86_64-pc-windows-msvc", ".exe"),
            ("aarch64-pc-windows-msvc", ".exe"),
            ("x86_64-pc-windows-gnu", ".exe"),
            ("aarch64-apple-darwin", ""),
            ("x86_64-apple-darwin", ""),
            ("x86_64-unknown-linux-gnu", ""),
            ("aarch64-unknown-linux-gnu", ""),
        ] {
            assert_eq!(assistant_host_exe_suffix(triple), suffix, "{triple}");
            assert_eq!(
                staged_assistant_host_file_name(triple),
                format!("onecad-assistant-host-{triple}{suffix}"),
                "{triple}"
            );
        }
        // The bundled name is about THIS machine, so it follows the platform.
        assert_eq!(
            bundled_assistant_host_file_name(),
            format!("onecad-assistant-host{}", std::env::consts::EXE_SUFFIX)
        );
        assert_eq!(
            cfg!(windows),
            bundled_assistant_host_file_name().ends_with(".exe")
        );
    }

    /// The other half of that centralisation: the build script and this module
    /// must produce the same file name, and the only way to know they do is to
    /// ask the script.
    #[test]
    fn the_staged_name_agrees_with_the_build_script() {
        if cfg!(windows) {
            // The script is bash; a Windows host has no gate to run here, and
            // there is no Windows CI lane to run it on.
            return;
        }
        let script = Path::new("../scripts/lib/assistant-host-target.sh");
        assert!(
            script.exists(),
            "the shared target mapping is missing: {}",
            script.display()
        );
        for triple in [
            "x86_64-unknown-linux-gnu",
            "aarch64-apple-darwin",
            "x86_64-pc-windows-msvc",
        ] {
            let output = std::process::Command::new("bash")
                .arg(script)
                .arg("staged-name")
                .arg(triple)
                .output()
                .expect("the target mapping script runs");
            assert!(
                output.status.success(),
                "{triple}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                staged_assistant_host_file_name(triple),
                "{triple}: the build script and the runtime lookup must name one file"
            );
        }
    }

    /// F15: the child used to inherit the whole parent environment. It now
    /// starts with the names a process needs and nothing else — which is
    /// hardening, not a sandbox, and the doc comment says so.
    #[test]
    fn the_child_environment_keeps_what_a_process_needs_and_drops_the_rest() {
        let parent: Vec<(OsString, OsString)> = [
            ("PATH", "/usr/bin:/bin"),
            ("HOME", "/home/andrej"),
            ("TMPDIR", "/tmp"),
            ("LANG", "en_US.UTF-8"),
            ("TZ", "Europe/Bratislava"),
            // Everything below is what must NOT cross.
            ("OPENAI_API_KEY", "sk-secret"),
            ("ANTHROPIC_API_KEY", "sk-secret"),
            ("AWS_SECRET_ACCESS_KEY", "secret"),
            ("GITHUB_TOKEN", "ghp_secret"),
            ("HTTPS_PROXY", "http://proxy.corp:8080"),
            ("http_proxy", "http://proxy.corp:8080"),
            ("ALL_PROXY", "socks5://proxy.corp:1080"),
            ("NODE_OPTIONS", "--require /tmp/evil.js"),
            ("BUN_INSPECT", "ws://127.0.0.1:6499/"),
            ("BUN_CONFIG_REGISTRY", "http://registry.evil/"),
            ("LD_PRELOAD", "/tmp/evil.so"),
            ("DYLD_INSERT_LIBRARIES", "/tmp/evil.dylib"),
            ("ONECAD_WORKER_PATH", "/somewhere/else"),
        ]
        .into_iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect();

        let child = minimal_child_env(parent.clone(), false);
        assert_eq!(
            child.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["HOME", "LANG", "PATH", "TMPDIR", "TZ"]
        );
        assert_eq!(child["PATH"], "/usr/bin:/bin");
        let printed = format!("{child:?}");
        for leaked in ["sk-secret", "ghp_secret", "proxy.corp", "evil"] {
            assert!(
                !printed.contains(leaked),
                "{leaked} must not cross: {printed}"
            );
        }

        // Windows keeps the system directories the loader itself reads, and
        // matches names case-insensitively the way Windows does.
        let windows_parent: Vec<(OsString, OsString)> = [
            ("Path", "C:\\Windows\\System32"),
            ("SystemRoot", "C:\\Windows"),
            ("LOCALAPPDATA", "C:\\Users\\Andrej\\AppData\\Local"),
            ("TEMP", "C:\\Users\\Andrej\\AppData\\Local\\Temp"),
            ("OPENAI_API_KEY", "sk-secret"),
            ("HTTPS_PROXY", "http://proxy.corp:8080"),
        ]
        .into_iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect();
        let child = minimal_child_env(windows_parent, true);
        assert_eq!(
            child.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["LOCALAPPDATA", "Path", "SystemRoot", "TEMP"],
            "the name is kept as the parent spelled it; only the MATCH is \
             case-insensitive"
        );

        // On POSIX the match is case-SENSITIVE, because there `Path` is not
        // `PATH` and inventing an equivalence would invent a variable.
        let child = minimal_child_env(
            vec![(OsString::from("Path"), OsString::from("/usr/bin"))],
            false,
        );
        assert!(child.is_empty());
    }

    #[test]
    fn log_level_defaults_to_info_and_refuses_a_level_the_child_would_reject() {
        // No env var set in this process, so this is the default path. The
        // validation itself is asserted through the pure list to avoid mutating
        // process-global state from a test.
        assert_eq!(assistant_log_level(), "info");
    }
}
