//! Bounded verified schemas retained for repeated live validation without ZIP decoding.
use super::*;
use crate::resource_admission::{ArchiveBudget, ArchiveUse, Retention};
use onecad_library::package::ComponentPackage;

const MAX_SCHEMA_BYTES: usize = 32 << 20;

struct SchemaEntry {
    package: Option<Arc<ComponentPackage>>,
    _retention: Retention,
}

#[derive(Default)]
pub(super) struct SchemaCache {
    schemas: BTreeMap<[u8; 32], SchemaEntry>,
    invalid_sources: BTreeMap<[u8; 32], Retention>,
    bytes: usize,
}

impl SchemaCache {
    fn charge(&mut self, budget: &ArchiveBudget, bytes: usize) -> Result<Retention, ApiError> {
        let total = self
            .bytes
            .checked_add(bytes)
            .filter(|total| *total <= MAX_SCHEMA_BYTES)
            .ok_or_else(|| {
                ApiError::InvalidCommand("component parameter schema budget exceeded".into())
            })?;
        let retention = budget
            .reserve(bytes, ArchiveUse::Retained)
            .map_err(ApiError::Io)?;
        self.bytes = total;
        Ok(retention)
    }

    pub(super) fn invalid_source(&self, source: &[u8; 32]) -> bool {
        self.invalid_sources.contains_key(source)
    }

    pub(super) fn remember_invalid(
        &mut self,
        budget: &ArchiveBudget,
        source: [u8; 32],
    ) -> Result<(), ApiError> {
        if !self.invalid_sources.contains_key(&source) {
            let retention = self.charge(budget, 128)?;
            self.invalid_sources.insert(source, retention);
        }
        Ok(())
    }

    pub(super) fn get(&self, key: &[u8; 32]) -> Option<Result<Arc<ComponentPackage>, ApiError>> {
        self.schemas.get(key).map(|entry| {
            entry.package.clone().ok_or_else(|| {
                ApiError::InvalidCommand(
                    "component defaults lack a verified generator snapshot".into(),
                )
            })
        })
    }

    pub(super) fn insert(
        &mut self,
        budget: &ArchiveBudget,
        key: [u8; 32],
        package: Option<ComponentPackage>,
    ) -> Result<Option<Arc<ComponentPackage>>, ApiError> {
        if let Some(existing) = self.schemas.get(&key) {
            return Ok(existing.package.clone());
        }
        let bytes = package.as_ref().map_or(Ok(128), schema_bytes)?;
        let retention = self.charge(budget, bytes)?;
        let package = package.map(Arc::new);
        self.schemas.insert(
            key,
            SchemaEntry {
                package: package.clone(),
                _retention: retention,
            },
        );
        Ok(package)
    }
}

fn schema_bytes(package: &ComponentPackage) -> Result<usize, ApiError> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .filter(|total| *total <= MAX_SCHEMA_BYTES / 32)
                .ok_or_else(|| {
                    std::io::Error::other("component parameter schema budget exceeded")
                })?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, package)
        .map_err(|error| ApiError::InvalidCommand(error.to_string()))?;
    counter
        .0
        .checked_mul(32)
        .and_then(|bytes| bytes.checked_add(4096))
        .filter(|bytes| *bytes <= MAX_SCHEMA_BYTES)
        .ok_or_else(|| {
            ApiError::InvalidCommand("component parameter schema budget exceeded".into())
        })
}

impl DocumentRuntime {
    pub(super) fn generator_schema(
        &self,
        params: &PlaceComponentParams,
    ) -> Result<Arc<ComponentPackage>, ApiError> {
        let key = fingerprint(&(
            &params.component_id,
            &params.component_version,
            &params.component_revision,
            self.parameter_archive_key(params)?,
        ));
        if let Some(cached) = self.parameter_repairs.schemas.lock().unwrap().get(&key) {
            return cached;
        }
        let archive = self.captured_library_archive(
            &params.component_id,
            &params.component_version,
            &params.component_revision,
        )?;
        let (package, _scratch, error) = match self.decode_generator_schema(params, archive) {
            Ok((package, scratch)) => (Some(package), Some(scratch), None),
            Err(error) => (None, None, Some(error)),
        };
        let package = self.parameter_repairs.schemas.lock().unwrap().insert(
            &self.archive_budget,
            key,
            package,
        )?;
        if let Some(error) = error {
            return Err(error);
        }
        package.ok_or_else(|| {
            ApiError::InvalidCommand("component generator snapshot unavailable".into())
        })
    }

    fn parameter_archive_key(
        &self,
        params: &PlaceComponentParams,
    ) -> Result<Option<&str>, ApiError> {
        let invalid = || ApiError::InvalidCommand("invalid component archive metadata".into());
        let document = self.session.document();
        let Some(state) = document
            .modules
            .iter()
            .find_map(|(id, state)| (id.as_str() == "onecad.library").then_some(state))
        else {
            return Ok(None);
        };
        if state.schema_version != 1 {
            return Err(invalid());
        }
        let object = state.payload.as_object().ok_or_else(invalid)?;
        let Some(snapshots) = object.get("snapshots") else {
            return Ok(None);
        };
        let snapshots = snapshots.as_object().ok_or_else(invalid)?;
        let Some(entry) = snapshots.get(&params.component_revision) else {
            return Ok(None);
        };
        if entry.get("componentId").and_then(serde_json::Value::as_str)
            != Some(&params.component_id)
            || entry
                .get("componentVersion")
                .and_then(serde_json::Value::as_str)
                != Some(&params.component_version)
        {
            return Err(invalid());
        }
        entry
            .get("archiveSha256")
            .and_then(serde_json::Value::as_str)
            .map(Some)
            .ok_or_else(invalid)
    }

    fn decode_generator_schema(
        &self,
        params: &PlaceComponentParams,
        archive: Option<crate::library_archive::ArchiveHandle>,
    ) -> Result<(ComponentPackage, Retention), ApiError> {
        let (reference, bytes) = archive.ok_or_else(|| {
            ApiError::InvalidCommand("component defaults require a verified snapshot".into())
        })?;
        let (snapshot, retention) = crate::library_archive::decode(
            &reference,
            &params.component_revision,
            &bytes,
            &self.archive_budget,
        )?;
        if !matches!(
            snapshot.package.source,
            onecad_library::package::SourceSpec::Generator { .. }
        ) {
            return Err(ApiError::InvalidCommand(
                "component source kind disagrees with verified snapshot".into(),
            ));
        }
        let package = snapshot.package;
        Ok((
            ComponentPackage {
                identity: package.identity,
                source: package.source,
                parameters: package.parameters,
                metadata: Default::default(),
                attachments: Default::default(),
                configurations: Default::default(),
            },
            retention,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_and_overflow_refuse_before_retaining_schema_entries() {
        for bytes in [MAX_SCHEMA_BYTES - 127, usize::MAX] {
            let mut cache = SchemaCache {
                bytes,
                ..Default::default()
            };
            assert!(cache
                .insert(&ArchiveBudget::default(), [0; 32], None)
                .is_err());
            assert!(cache.schemas.is_empty());
            assert_eq!(cache.bytes, bytes);
        }
    }

    #[test]
    fn denied_host_reservation_does_not_retain_schema_or_invalid_source() {
        struct Deny;
        impl crate::resource_admission::ArchiveAdmission for Deny {
            fn reserve(&self, _: usize, _: ArchiveUse) -> Result<Retention, String> {
                Err("schema budget denied".into())
            }
        }
        let budget = ArchiveBudget::new(Arc::new(Deny));
        let mut cache = SchemaCache::default();
        assert!(cache.insert(&budget, [0; 32], None).is_err());
        assert!(cache.remember_invalid(&budget, [1; 32]).is_err());
        assert!(cache.schemas.is_empty());
        assert!(cache.invalid_sources.is_empty());
        assert_eq!(cache.bytes, 0);
    }
}
