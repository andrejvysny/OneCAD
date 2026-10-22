//! Worker release identity checks shared by both backends: the hello axes a
//! manifest pins, and the embedded kernel's manifest v2
//! (`onecad-embedded-kernel.json`, docs/ios/KERNEL.md).

use onecad_protocol::messages::HelloResult;

/// The hello axes a release identity pins — shared by the sidecar manifest (v1)
/// and the embedded kernel identity (v2), so both are checked by one rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HelloAxes {
    pub(crate) protocol_version: u32,
    pub(crate) worker_version: String,
    pub(crate) quantization_version: u32,
    pub(crate) solver_policy_version: u32,
    pub(crate) occt_version: String,
    pub(crate) occt_fingerprint: String,
}

pub(crate) fn verify_hello_axes(expected: &HelloAxes, actual: &HelloResult) -> Result<(), String> {
    compare(
        "protocolVersion",
        expected.protocol_version,
        actual.protocol_version,
    )?;
    compare(
        "quantizationVersion",
        expected.quantization_version,
        actual.quantization_version,
    )?;
    compare(
        "solverPolicyVersion",
        expected.solver_policy_version,
        actual.solver_policy_version,
    )?;
    compare_ref(
        "workerVersion",
        &expected.worker_version,
        &actual.worker_version,
    )?;
    compare_ref("occt.version", &expected.occt_version, &actual.occt.version)?;
    compare_ref(
        "occt.fingerprint",
        &expected.occt_fingerprint,
        &actual.occt.fingerprint,
    )
}

#[cfg_attr(
    not(any(feature = "sidecar", feature = "embedded-kernel")),
    allow(dead_code)
)]
pub(crate) fn validate_hex(label: &str, value: &str, length: usize) -> Result<(), String> {
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "worker manifest {label} must be {length} lowercase hex characters"
        ));
    }
    Ok(())
}

fn compare<T>(label: &str, expected: T, actual: T) -> Result<(), String>
where
    T: PartialEq + std::fmt::Display,
{
    if actual != expected {
        return Err(format!(
            "worker hello {label} mismatch: expected {expected}, got {actual}"
        ));
    }
    Ok(())
}

fn compare_ref<T>(label: &str, expected: &T, actual: &T) -> Result<(), String>
where
    T: PartialEq + std::fmt::Display,
{
    if actual != expected {
        return Err(format!(
            "worker hello {label} mismatch: expected {expected}, got {actual}"
        ));
    }
    Ok(())
}

#[cfg(feature = "embedded-kernel")]
pub(crate) use embedded::parse_embedded_identity;

#[cfg(feature = "embedded-kernel")]
mod embedded {
    use serde::Deserialize;

    use super::{validate_hex, HelloAxes};

    /// `onecad-embedded-kernel.json`: manifest v2, `kind: "embedded"`.
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(crate) struct EmbeddedIdentity {
        manifest_version: u32,
        kind: String,
        pub(crate) abi_version: i32,
        protocol_version: u32,
        worker_version: String,
        quantization_version: u32,
        solver_policy_version: u32,
        occt: EmbeddedOcct,
        pub(crate) platform: String,
        sdk_version: String,
        deployment_target: String,
        archive_sha256: String,
        pub(crate) test_hooks: bool,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct EmbeddedOcct {
        version: String,
        build_id: String,
        fingerprint: String,
    }

    const PLATFORMS: [&str; 3] = ["iphonesimulator", "iphoneos", "macosx"];

    pub(crate) fn parse_embedded_identity(json: &str) -> Result<EmbeddedIdentity, String> {
        let identity: EmbeddedIdentity = serde_json::from_str(json)
            .map_err(|error| format!("invalid embedded kernel identity: {error}"))?;
        if identity.manifest_version != 2 || identity.kind != "embedded" {
            return Err(format!(
                "unsupported embedded kernel identity (manifestVersion {}, kind {:?})",
                identity.manifest_version, identity.kind
            ));
        }
        if !PLATFORMS.contains(&identity.platform.as_str()) {
            return Err(format!(
                "embedded kernel identity names unknown platform {:?}",
                identity.platform
            ));
        }
        validate_hex("OCCT fingerprint", &identity.occt.fingerprint, 16)?;
        validate_hex("archive SHA-256", &identity.archive_sha256, 64)?;
        if identity.occt.build_id.is_empty()
            || identity.sdk_version.is_empty()
            || identity.deployment_target.is_empty()
        {
            return Err("embedded kernel identity has an empty build field".into());
        }
        Ok(identity)
    }

    impl EmbeddedIdentity {
        pub(crate) fn axes(&self) -> HelloAxes {
            HelloAxes {
                protocol_version: self.protocol_version,
                worker_version: self.worker_version.clone(),
                quantization_version: self.quantization_version,
                solver_policy_version: self.solver_policy_version,
                occt_version: self.occt.version.clone(),
                occt_fingerprint: self.occt.fingerprint.clone(),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const SAMPLE: &str = r#"{
            "manifestVersion": 2, "kind": "embedded", "abiVersion": 3,
            "protocolVersion": 1, "workerVersion": "0.1.0",
            "quantizationVersion": 1, "solverPolicyVersion": 4,
            "occt": { "version": "8.0.1",
                      "buildId": "onecad-occt-8.0.1-b8f597c67781-kp1-iphonesimulator-arm64-ios18.0",
                      "fingerprint": "1235abe6387959f8" },
            "platform": "iphonesimulator", "sdkVersion": "26.5", "deploymentTarget": "18.0",
            "archiveSha256": "650761af6ba4b93ad866510cda8854192aeb25e88f9c122e3b99b323ae76513b",
            "testHooks": false
        }"#;

        #[test]
        fn parses_the_v2_shape_and_exposes_its_hello_axes() {
            let identity = parse_embedded_identity(SAMPLE).unwrap();
            assert_eq!(identity.abi_version, 3);
            assert_eq!(identity.platform, "iphonesimulator");
            assert!(!identity.test_hooks);
            assert_eq!(identity.axes().occt_fingerprint, "1235abe6387959f8");
            assert_eq!(identity.axes().solver_policy_version, 4);
        }

        #[test]
        fn rejects_v1_unknown_fields_bad_platforms_and_digests() {
            let cases = [
                (
                    SAMPLE.replace("\"manifestVersion\": 2", "\"manifestVersion\": 1"),
                    "unsupported",
                ),
                (SAMPLE.replace("\"embedded\"", "\"sidecar\""), "unsupported"),
                (
                    SAMPLE.replace("\"iphonesimulator\",", "\"android\","),
                    "unknown platform",
                ),
                (
                    SAMPLE.replace("1235abe6387959f8", "XYZ"),
                    "OCCT fingerprint",
                ),
                (SAMPLE.replace("650761af", "650761aF"), "archive SHA-256"),
                (
                    SAMPLE.replace("\"testHooks\"", "\"binarySha256\": \"x\", \"testHooks\""),
                    "unknown field",
                ),
            ];
            for (json, needle) in cases {
                let error = parse_embedded_identity(&json).unwrap_err();
                assert!(error.contains(needle), "{needle}: {error}");
            }
        }
    }
}
