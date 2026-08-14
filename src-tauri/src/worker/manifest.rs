use std::fs::File;
use std::io::Read;
use std::path::Path;

use onecad_protocol::messages::HelloResult;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const EMBEDDED_MANIFEST: &str =
    include_str!(concat!(env!("OUT_DIR"), "/onecad-worker-manifest.json"));

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct WorkerManifest {
    manifest_version: u32,
    binary_sha256: String,
    protocol_version: u32,
    worker_version: String,
    quantization_version: u32,
    solver_policy_version: u32,
    occt: ManifestOcct,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManifestOcct {
    version: String,
    fingerprint: String,
}

pub(super) fn embedded_release_manifest() -> Result<Option<WorkerManifest>, String> {
    if cfg!(debug_assertions) {
        return Ok(None);
    }
    parse_manifest(EMBEDDED_MANIFEST).map(Some)
}

fn parse_manifest(json: &str) -> Result<WorkerManifest, String> {
    let manifest: WorkerManifest =
        serde_json::from_str(json).map_err(|error| format!("invalid worker manifest: {error}"))?;
    if manifest.manifest_version != 1 {
        return Err(format!(
            "unsupported worker manifest version {}",
            manifest.manifest_version
        ));
    }
    validate_hex("binary SHA-256", &manifest.binary_sha256, 64)?;
    validate_hex("OCCT fingerprint", &manifest.occt.fingerprint, 16)?;
    Ok(manifest)
}

pub(super) fn verify_binary(path: &Path, expected: &WorkerManifest) -> Result<(), String> {
    let mut file = File::open(path)
        .map_err(|error| format!("open bundled worker {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut chunk)
            .map_err(|error| format!("hash bundled worker {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        hasher.update(&chunk[..count]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected.binary_sha256 {
        return Err(format!(
            "bundled worker SHA-256 mismatch: expected {}, got {actual}",
            expected.binary_sha256
        ));
    }
    Ok(())
}

pub(super) fn verify_hello(expected: &WorkerManifest, actual: &HelloResult) -> Result<(), String> {
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
    compare_ref("occt.version", &expected.occt.version, &actual.occt.version)?;
    compare_ref(
        "occt.fingerprint",
        &expected.occt.fingerprint,
        &actual.occt.fingerprint,
    )
}

fn validate_hex(label: &str, value: &str, length: usize) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use onecad_protocol::messages::{HelloLimits, OcctInfo};

    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn manifest(binary_sha256: &str) -> WorkerManifest {
        parse_manifest(&format!(
            r#"{{
                "manifestVersion": 1,
                "binarySha256": "{binary_sha256}",
                "protocolVersion": 1,
                "workerVersion": "0.1.0",
                "quantizationVersion": 1,
                "solverPolicyVersion": 1,
                "occt": {{ "version": "8.0.1", "fingerprint": "0a6a1dce34181289" }}
            }}"#
        ))
        .unwrap()
    }

    fn hello() -> HelloResult {
        HelloResult {
            protocol_version: 1,
            worker_version: "0.1.0".into(),
            occt: OcctInfo {
                version: "8.0.1".into(),
                fingerprint: "0a6a1dce34181289".into(),
            },
            quantization_version: 1,
            solver_policy_version: 1,
            capabilities: Vec::new(),
            limits: Some(HelloLimits {
                chunk_size: 1_048_576,
                initial_bulk_credit: 8_388_608,
            }),
        }
    }

    #[test]
    fn rejects_invalid_manifest_shape_and_digests() {
        assert!(parse_manifest("null").is_err());
        assert!(parse_manifest(
            r#"{"manifestVersion":2,"binarySha256":"00","protocolVersion":1,
            "workerVersion":"x","quantizationVersion":1,"solverPolicyVersion":1,
            "occt":{"version":"x","fingerprint":"00"}}"#
        )
        .unwrap_err()
        .contains("unsupported worker manifest version"));
        assert!(parse_manifest(
            r#"{"manifestVersion":1,"binarySha256":"00","protocolVersion":1,
            "workerVersion":"x","quantizationVersion":1,"solverPolicyVersion":1,
            "occt":{"version":"x","fingerprint":"0000000000000000"}}"#
        )
        .unwrap_err()
        .contains("binary SHA-256"));
    }

    #[test]
    fn verifies_binary_sha_before_spawn() {
        let dir = tempfile::tempdir().unwrap();
        let worker = dir.path().join("onecad-worker");
        std::fs::write(&worker, b"worker bytes").unwrap();
        let digest = format!("{:x}", Sha256::digest(b"worker bytes"));
        verify_binary(&worker, &manifest(&digest)).unwrap();

        let error = verify_binary(&worker, &manifest(SHA)).unwrap_err();
        assert!(error.contains("SHA-256 mismatch"), "{error}");
    }

    #[test]
    fn rejects_each_hello_lockstep_axis() {
        let expected = manifest(SHA);
        verify_hello(&expected, &hello()).unwrap();

        let cases: Vec<(&str, HelloResult)> = vec![
            (
                "protocolVersion",
                HelloResult {
                    protocol_version: 2,
                    ..hello()
                },
            ),
            (
                "quantizationVersion",
                HelloResult {
                    quantization_version: 2,
                    ..hello()
                },
            ),
            (
                "solverPolicyVersion",
                HelloResult {
                    solver_policy_version: 2,
                    ..hello()
                },
            ),
            (
                "workerVersion",
                HelloResult {
                    worker_version: "0.2.0".into(),
                    ..hello()
                },
            ),
            (
                "occt.version",
                HelloResult {
                    occt: OcctInfo {
                        version: "7.9.3".into(),
                        ..hello().occt
                    },
                    ..hello()
                },
            ),
            (
                "occt.fingerprint",
                HelloResult {
                    occt: OcctInfo {
                        fingerprint: "1111111111111111".into(),
                        ..hello().occt
                    },
                    ..hello()
                },
            ),
        ];
        for (label, actual) in cases {
            let error = verify_hello(&expected, &actual).unwrap_err();
            assert!(error.contains(label), "{label}: {error}");
        }
    }
}
