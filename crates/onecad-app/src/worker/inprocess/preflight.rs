//! What [`InProcessHost::launch`](super::InProcessHost) checks before any thread
//! exists: the ABI, which identity manifest is verified (and its platform and
//! test hooks), and — for an iOS release — that the crash journal is configured
//! (docs/ios/KERNEL.md "Verification at start").

use std::path::Path;

use super::super::identity::{parse_embedded_identity, HelloAxes};

/// The `platform` the identity of a kernel linked into THIS binary must name.
const TARGET_PLATFORM: &str = if cfg!(all(target_os = "ios", target_abi = "sim")) {
    "iphonesimulator"
} else if cfg!(target_os = "ios") {
    "iphoneos"
} else {
    "macosx"
};

/// The build's rules: debug builds accept a test identity override and run
/// without an identity; a release iOS build also requires the crash journal and
/// refuses a test-hooks kernel.
#[derive(Debug, Clone, Copy)]
pub(super) struct Policy {
    pub(super) debug: bool,
    pub(super) release_ios: bool,
    pub(super) platform: &'static str,
}

impl Policy {
    pub(super) const BUILD: Policy = Policy {
        debug: cfg!(debug_assertions),
        release_ios: cfg!(all(target_os = "ios", not(debug_assertions))),
        platform: TARGET_PLATFORM,
    };
}

/// Everything one launch is checked against.
pub(super) struct Inputs<'a> {
    /// `onecad_worker_abi_version()` of the linked library.
    pub(super) abi: i32,
    /// `EmbeddedKernelConfig::identity` (a test override).
    pub(super) explicit: Option<&'a str>,
    /// The copy `onecad-app/build.rs` embedded.
    pub(super) app: Option<&'a str>,
    /// The copy the link crate embedded with the bundle it linked.
    pub(super) linked: Option<&'a str>,
    pub(super) journal_dir: Option<&'a Path>,
}

/// ABI first, then the journal rule, then the identity manifest. Returns the
/// hello axes the handshake must match (`None`: a debug build without one).
pub(super) fn check(inputs: &Inputs<'_>, policy: Policy) -> Result<Option<HelloAxes>, String> {
    let abi = inputs.abi;
    if abi != onecad_embedded_worker::ABI_VERSION {
        return Err(format!(
            "embedded kernel ABI {abi}; this host speaks ABI {}",
            onecad_embedded_worker::ABI_VERSION
        ));
    }
    if policy.release_ios && inputs.journal_dir.is_none() {
        return Err(
            "an iOS release requires the kernel crash journal: configure \
                    EmbeddedKernelConfig::for_app_support(<app_support>)"
                .into(),
        );
    }
    let Some(text) = select_identity(inputs, policy.debug)? else {
        if policy.debug {
            tracing::warn!(
                "embedded kernel has no identity manifest (debug build): hello unchecked"
            );
            return Ok(None);
        }
        return Err("release build without an embedded kernel identity".into());
    };
    let identity = parse_embedded_identity(&text)?;
    if identity.abi_version != abi {
        return Err(format!(
            "embedded kernel identity names ABI {}, the linked library is ABI {abi}",
            identity.abi_version
        ));
    }
    if identity.platform != policy.platform {
        return Err(format!(
            "embedded kernel identity names platform {:?}; this binary runs on {:?}",
            identity.platform, policy.platform
        ));
    }
    if policy.release_ios && identity.test_hooks {
        return Err("an iOS release must not run a kernel built with test hooks".into());
    }
    Ok(Some(identity.axes()))
}

/// The identity text to verify: the app's embedded copy, which must equal the
/// linked bundle's own. An explicit override is honoured in debug builds only
/// (tests force mismatches with it); a release verifies the kernel it links.
fn select_identity(inputs: &Inputs<'_>, debug: bool) -> Result<Option<String>, String> {
    if let (Some(app), Some(linked)) = (inputs.app, inputs.linked) {
        if app != linked {
            return Err(
                "onecad-app and onecad-embedded-worker embed different kernel \
                        identities (stale build: rebuild with one ONECAD_EMBEDDED_WORKER_DIR)"
                    .into(),
            );
        }
    }
    match inputs.explicit {
        None => Ok(inputs.app.map(str::to_owned)),
        Some(text) if debug || Some(text) == inputs.linked => Ok(Some(text.to_owned())),
        Some(_) => Err(
            "an embedded kernel identity override that is not the linked \
                        kernel's own is refused outside debug builds"
                .into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    const SIM: &str = r#"{
        "manifestVersion": 2, "kind": "embedded", "abiVersion": 3,
        "protocolVersion": 1, "workerVersion": "0.1.0",
        "quantizationVersion": 1, "solverPolicyVersion": 4,
        "occt": { "version": "8.0.1", "buildId": "b", "fingerprint": "1235abe6387959f8" },
        "platform": "iphonesimulator", "sdkVersion": "26.5", "deploymentTarget": "18.0",
        "archiveSha256": "650761af6ba4b93ad866510cda8854192aeb25e88f9c122e3b99b323ae76513b",
        "testHooks": false
    }"#;

    const RELEASE_SIM: Policy = Policy {
        debug: false,
        release_ios: true,
        platform: "iphonesimulator",
    };

    fn inputs<'a>(identity: &'a str, journal: Option<&'a Path>) -> Inputs<'a> {
        Inputs {
            abi: 3,
            explicit: None,
            app: Some(identity),
            linked: Some(identity),
            journal_dir: journal,
        }
    }

    fn refused(inputs: &Inputs<'_>, policy: Policy) -> String {
        check(inputs, policy).expect_err("refused")
    }

    #[test]
    fn a_release_ios_launch_needs_the_journal_and_the_target_platform() {
        let journal = Path::new("/support/kernel-journal");
        assert!(check(&inputs(SIM, Some(journal)), RELEASE_SIM)
            .unwrap()
            .is_some());
        assert!(refused(&inputs(SIM, None), RELEASE_SIM).contains("crash journal"));
        let device = Policy {
            platform: "iphoneos",
            ..RELEASE_SIM
        };
        assert!(refused(&inputs(SIM, Some(journal)), device).contains("platform"));
        let hooks = SIM.replace("\"testHooks\": false", "\"testHooks\": true");
        assert!(refused(&inputs(&hooks, Some(journal)), RELEASE_SIM).contains("test hooks"));
        let debug = Policy {
            debug: true,
            release_ios: false,
            ..RELEASE_SIM
        };
        assert!(
            check(&inputs(SIM, None), debug).unwrap().is_some(),
            "debug: journal optional"
        );
    }

    #[test]
    fn an_override_counts_in_debug_only_and_copies_must_agree() {
        let other = SIM.replace("1235abe6387959f8", "0000000000000000");
        let mut forced = inputs(SIM, Some(Path::new("/j")));
        forced.explicit = Some(&other);
        assert!(refused(&forced, RELEASE_SIM).contains("override"));
        let debug = Policy {
            debug: true,
            ..RELEASE_SIM
        };
        let axes = check(&forced, debug).unwrap().unwrap();
        assert_eq!(axes.occt_fingerprint, "0000000000000000");
        let mut stale = inputs(SIM, Some(Path::new("/j")));
        stale.linked = Some(&other);
        assert!(refused(&stale, debug).contains("stale build"));
        let mut abi = inputs(SIM, Some(Path::new("/j")));
        abi.abi = 4;
        assert!(refused(&abi, debug).contains("ABI 4"));
        let none = Inputs {
            app: None,
            linked: None,
            ..inputs(SIM, Some(Path::new("/j")))
        };
        assert!(
            check(&none, debug).unwrap().is_none(),
            "debug without identity"
        );
        assert!(refused(&none, RELEASE_SIM).contains("without an embedded kernel identity"));
    }
}
