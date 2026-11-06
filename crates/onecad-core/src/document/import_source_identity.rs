//! Critical source-addressed identity for imported topology.
use serde::{Deserialize, Serialize};

use crate::ids::{BodyId, RecordId};

/// Exact source identity carried by a durable imported face reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "ImportSourceIdentityWire")]
pub struct ImportSourceIdentity {
    pub version: u32,
    pub source_sha256: String,
    pub codec: ImportIdentityCodec,
    pub codec_version: String,
    pub reader_policy: String,
    pub kernel_fingerprint: String,
    pub producer_record_id: RecordId,
    pub body_id: BodyId,
    pub unit_scale_bits: String,
    pub source_address: ImportSourceAddress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportIdentityCodec {
    Step,
    Xbf,
}

/// Source address; unsupported or unproven address forms are refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum ImportSourceAddress {
    Step {
        #[serde(rename = "rootEntityLabel")]
        root_entity_label: u64,
        #[serde(rename = "occurrencePath")]
        occurrence_path: Vec<u64>,
        #[serde(rename = "faceEntityLabel")]
        face_entity_label: u64,
    },
    Xbf {
        #[serde(rename = "rootLabel")]
        root_label: String,
        #[serde(rename = "occurrencePath")]
        occurrence_path: Vec<String>,
        #[serde(rename = "faceLabel")]
        face_label: String,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ImportSourceIdentityWire {
    version: u32,
    source_sha256: String,
    codec: ImportIdentityCodec,
    codec_version: String,
    reader_policy: String,
    kernel_fingerprint: String,
    producer_record_id: RecordId,
    body_id: BodyId,
    unit_scale_bits: String,
    source_address: ImportSourceAddress,
}

fn lowercase_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn valid_label(value: &str) -> bool {
    value.len() <= 256
        && value.starts_with("0:")
        && value
            .split(':')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

impl ImportSourceIdentity {
    /// Refuse malformed or unimplemented critical identity semantics.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != 1 {
            return Err("unsupported import source identity version");
        }
        if !lowercase_hex(&self.source_sha256, 64) || !lowercase_hex(&self.unit_scale_bits, 16) {
            return Err("invalid import source identity digest or scale");
        }
        let scale = u64::from_str_radix(&self.unit_scale_bits, 16)
            .map(f64::from_bits)
            .map_err(|_| "invalid import source identity scale")?;
        if !scale.is_finite() || scale <= 0.0 || !lowercase_hex(&self.kernel_fingerprint, 16) {
            return Err("invalid import source identity scale or kernel fingerprint");
        }
        match (&self.codec, &self.source_address) {
            (
                ImportIdentityCodec::Step,
                ImportSourceAddress::Step {
                    root_entity_label,
                    occurrence_path,
                    face_entity_label,
                },
            ) if self.codec_version == "occt-step-stream-v1"
                && self.reader_policy == "step-v1"
                && *root_entity_label > 0
                && *face_entity_label > 0
                && occurrence_path.is_empty() =>
            {
                Ok(())
            }
            (
                ImportIdentityCodec::Xbf,
                ImportSourceAddress::Xbf {
                    root_label,
                    occurrence_path,
                    face_label,
                },
            ) if self.codec_version == "12"
                && self.reader_policy == "xbf-v12"
                && occurrence_path.is_empty()
                && valid_label(root_label)
                && valid_label(face_label)
                && face_label
                    .strip_prefix(root_label)
                    .is_some_and(|suffix| suffix.starts_with(':')) =>
            {
                Ok(())
            }
            _ => Err("unsupported import source address or policy"),
        }
    }
}

impl TryFrom<ImportSourceIdentityWire> for ImportSourceIdentity {
    type Error = &'static str;
    fn try_from(wire: ImportSourceIdentityWire) -> Result<Self, Self::Error> {
        let value = Self {
            version: wire.version,
            source_sha256: wire.source_sha256,
            codec: wire.codec,
            codec_version: wire.codec_version,
            reader_policy: wire.reader_policy,
            kernel_fingerprint: wire.kernel_fingerprint,
            producer_record_id: wire.producer_record_id,
            body_id: wire.body_id,
            unit_scale_bits: wire.unit_scale_bits,
            source_address: wire.source_address,
        };
        value.validate()?;
        Ok(value)
    }
}

/// Deserialize an explicitly present certificate; JSON null is not absence.
pub fn deserialize_present<'de, D>(
    deserializer: D,
) -> Result<Option<ImportSourceIdentity>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    ImportSourceIdentity::deserialize(deserializer).map(Some)
}

/// Validate a reference against its authoritative definition-local element index.
pub fn validate_reference(
    reference: &super::refs::ElementRef,
    entries: &super::element_index::ElementIndex,
) -> Result<(), String> {
    let Some(primary) = &reference.primary else {
        return if reference.import_source_identity.is_some() {
            Err("certified element reference has no primary identity".into())
        } else {
            Ok(())
        };
    };
    if let Some(cert) = &reference.import_source_identity {
        cert.validate().map_err(str::to_string)?;
        if cert.body_id != primary.body || primary.kind != super::refs::ElementKind::Face {
            return Err("certified element reference body or kind mismatch".into());
        }
    }
    if let Some(entry) = entries.get(&primary.element) {
        if entry.import_source_identity != reference.import_source_identity {
            return Err("element reference and index import source identities disagree".into());
        }
    }
    Ok(())
}

pub(crate) fn validate_definition(
    definition: &mut super::component::ComponentDef,
) -> Result<(), String> {
    let entries = &definition.elements;
    let mut certificates = Vec::new();
    for (id, entry) in entries.iter() {
        if let Some(cert) = &entry.import_source_identity {
            cert.validate().map_err(str::to_string)?;
            if entry.body != cert.body_id || entry.kind != super::refs::ElementKind::Face {
                return Err("certified element index body or kind mismatch".into());
            }
            if certificates
                .iter()
                .any(|(other_id, other)| other == &cert && *other_id != id)
            {
                return Err("competing element ids share import source identity".into());
            }
            certificates.push((id, cert));
        }
    }
    for sketch in definition.sketches.values() {
        if let Some(face) = sketch.host_face() {
            validate_reference(face, entries)?;
        }
    }
    for record in definition.timeline.effective_records_mut() {
        if let super::record::Operation::Known(operation) = &mut record.op {
            for reference in operation.stored_element_refs_mut() {
                validate_reference(reference, entries)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn certificate() -> serde_json::Value {
        json!({"version":1,"sourceSha256":"a".repeat(64),"codec":"step", "codecVersion":"occt-step-stream-v1",
            "readerPolicy":"step-v1", "kernelFingerprint":"0a6a1dce34181289", "producerRecordId":"00000000-0000-0000-0000-000000000001",
            "bodyId":"00000000-0000-0000-0000-000000000002", "unitScaleBits":"3ff0000000000000",
            "sourceAddress":{"kind":"step", "rootEntityLabel":5850, "occurrencePath":[], "faceEntityLabel":3042}})
    }

    #[test]
    fn xbf_certificate_requires_explicit_flat_persisted_labels() {
        let mut value = certificate();
        value["codec"] = json!("xbf");
        value["codecVersion"] = json!("12");
        value["readerPolicy"] = json!("xbf-v12");
        value["sourceAddress"] = json!({"kind":"xbf","rootLabel":"0:1:1:1",
            "occurrencePath":[],"faceLabel":"0:1:1:1:7"});
        assert!(serde_json::from_value::<ImportSourceIdentity>(value.clone()).is_ok());
        value["sourceAddress"]["faceLabel"] = json!("0:1:1:2:7");
        assert!(serde_json::from_value::<ImportSourceIdentity>(value).is_err());
    }

    #[test]
    fn accepts_exact_source_address_and_rejects_unknown_meaning() {
        let valid = certificate();
        let value: ImportSourceIdentity = serde_json::from_value(valid.clone()).unwrap();
        assert_eq!(serde_json::to_value(value).unwrap(), valid);
        for (key, replacement) in [
            ("version", json!(2)),
            ("sourceSha256", json!("bad")),
            ("codec", json!("brep")),
            ("unitScaleBits", json!("7ff0000000000000")),
            ("extra", json!(true)),
        ] {
            let mut invalid = valid.clone();
            invalid[key] = replacement;
            assert!(
                serde_json::from_value::<ImportSourceIdentity>(invalid).is_err(),
                "{key}"
            );
        }
        let mut nested = valid;
        nested["sourceAddress"]["occurrencePath"] = json!([4]);
        assert!(serde_json::from_value::<ImportSourceIdentity>(nested).is_err());
    }
}
