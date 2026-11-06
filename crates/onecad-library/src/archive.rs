//! Deterministic, bounded transport for one verified semantic package closure.
//!
//! Entry names are fixed independently of package paths. The manifest retains its
//! original source filename; the document and blob bytes live in fixed entries.
use std::io::{Cursor, Read, Seek, Write};
use std::path::{Component, Path};

use crate::error::{LibraryError, LibraryResult};
use crate::package::{self, SourceSpec};
use crate::resolve::{ResolveRequest, ResolvedBlob, ResolvedSnapshot, ResolvedSource};

const MANIFEST: &str = "component.toml";
const DOCUMENT: &str = "document.bin";
const BLOB: &str = "geometry.bin";
const HARD_LIMIT: u64 = crate::blob::MAX_BLOB_BYTES;

fn invalid(message: &str) -> LibraryError {
    LibraryError::InvalidIdentity(format!("component archive: {message}"))
}

fn checked_limit(size: u64, limit: u64) -> LibraryResult<()> {
    let limit = limit.min(HARD_LIMIT);
    if size > limit {
        return Err(LibraryError::BlobTooLarge {
            limit,
            actual: size,
        });
    }
    Ok(())
}

/// Fixed-capacity seekable output whose end tracks written bytes, not buffer capacity.
pub struct BoundedWriter<'a> {
    buffer: &'a mut [u8],
    position: usize,
    used: usize,
}
impl<'a> BoundedWriter<'a> {
    pub fn new(buffer: &'a mut [u8]) -> Self {
        Self {
            buffer,
            position: 0,
            used: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.used
    }
    pub fn is_empty(&self) -> bool {
        self.used == 0
    }
}
impl Write for BoundedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let end = self
            .position
            .checked_add(bytes.len())
            .filter(|end| *end <= self.buffer.len())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "archive exceeded admitted buffer",
                )
            })?;
        self.buffer[self.position..end].copy_from_slice(bytes);
        self.position = end;
        self.used = self.used.max(end);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Seek for BoundedWriter<'_> {
    fn seek(&mut self, origin: std::io::SeekFrom) -> std::io::Result<u64> {
        let pos = match origin {
            std::io::SeekFrom::Start(n) => i128::from(n),
            std::io::SeekFrom::End(n) => self.used as i128 + i128::from(n),
            std::io::SeekFrom::Current(n) => self.position as i128 + i128::from(n),
        };
        let pos = usize::try_from(pos).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid archive seek")
        })?;
        if pos > self.used {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "archive seek beyond written bytes",
            ));
        }
        self.position = pos;
        Ok(pos as u64)
    }
}

/// Encode a captured closure without consulting the library or filesystem.
/// `None` produces no archive. Limits apply to the whole ZIP and never exceed
/// the library's existing 256 MiB payload bound.
pub fn encode(snapshot: Option<&ResolvedSnapshot>, limit: u64) -> LibraryResult<Option<Vec<u8>>> {
    let Some(snapshot) = snapshot else {
        return Ok(None);
    };
    let mut output = Cursor::new(Vec::new());
    encode_into(snapshot, limit, &mut output)?;
    Ok(Some(output.into_inner()))
}

/// Encodes into caller-owned bounded storage, without allocating an output copy.
pub fn encode_into<W: Write + Seek>(
    snapshot: &ResolvedSnapshot,
    limit: u64,
    output: W,
) -> LibraryResult<W> {
    let blob = source_bytes(&snapshot.source);
    let size = (snapshot.manifest.len() as u64)
        .checked_add(
            snapshot
                .document
                .as_ref()
                .map_or(0, |data| data.len() as u64),
        )
        .and_then(|size| size.checked_add(blob.map_or(0, |data| data.len() as u64)))
        .ok_or_else(|| invalid("payload size overflow"))?;
    checked_limit(size, limit)?;
    validate_snapshot(snapshot)?;
    let mut writer = zip::ZipWriter::new(output);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    for (name, data) in [
        (MANIFEST, Some(snapshot.manifest.as_bytes())),
        (DOCUMENT, snapshot.document.as_deref()),
        (BLOB, blob),
    ] {
        if let Some(data) = data {
            writer
                .start_file(name, options)
                .map_err(|_| invalid("cannot write ZIP entry"))?;
            writer.write_all(data)?;
        }
    }
    let mut output = writer.finish().map_err(|_| invalid("cannot finish ZIP"))?;
    checked_limit(output.stream_position()?, limit)?;
    Ok(output)
}

/// Verify archive bytes, exact component identity, semantic revision and payload closure.
/// `archive_sha256` is the bare SHA-256 of the complete ZIP supplied by its carrier.
pub fn decode(
    bytes: &[u8],
    archive_sha256: &str,
    expected: &ResolveRequest<'_>,
    limit: u64,
) -> LibraryResult<ResolvedSnapshot> {
    checked_limit(bytes.len() as u64, limit)?;
    if crate::blob::sha256_hex(bytes) != archive_sha256 {
        return Err(invalid("archive hash mismatch"));
    }
    let central_start = validate_directory(bytes)?;
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| invalid("invalid ZIP"))?;
    if zip.is_empty() || zip.len() > 3 {
        return Err(invalid("unexpected entry count"));
    }
    let mut entries: [Option<Vec<u8>>; 3] = [None, None, None];
    let mut total = 0u64;
    let mut local_end = 0u64;
    let mut previous_slot = None;
    for index in 0..zip.len() {
        let mut file = zip
            .by_index(index)
            .map_err(|_| invalid("invalid ZIP entry"))?;
        let slot = match file.name() {
            MANIFEST => 0,
            DOCUMENT => 1,
            BLOB => 2,
            _ => return Err(invalid("unexpected ZIP entry")),
        };
        if previous_slot.is_some_and(|previous| slot <= previous)
            || file.header_start() != local_end
        {
            return Err(invalid("noncanonical entry order or layout"));
        }
        previous_slot = Some(slot);
        local_end = file
            .data_start()
            .checked_add(file.compressed_size())
            .ok_or_else(|| invalid("entry extent overflow"))?;
        if entries[slot].is_some()
            || file.compression() != zip::CompressionMethod::Stored
            || file.is_dir()
        {
            return Err(invalid("duplicate or unsupported ZIP entry"));
        }
        total = total
            .checked_add(file.size())
            .ok_or_else(|| invalid("entry size overflow"))?;
        checked_limit(total, limit)?;
        let size = usize::try_from(file.size()).map_err(|_| invalid("entry size overflow"))?;
        // SAFETY: every byte is initialized to zero, and all bit patterns are valid for u8.
        let mut data = unsafe { Box::<[u8]>::new_zeroed_slice(size).assume_init() };
        file.read_exact(&mut data)?;
        let mut probe = [0u8; 1];
        if file.read(&mut probe)? != 0 {
            return Err(invalid("entry length mismatch"));
        }
        entries[slot] = Some(data.into_vec());
    }
    if local_end != central_start {
        return Err(invalid("unexpected ZIP data"));
    }
    let [manifest, document, blob] = entries;
    let manifest = String::from_utf8(manifest.ok_or_else(|| invalid("missing manifest"))?)
        .map_err(|_| invalid("manifest is not UTF-8"))?;
    verify(&manifest, document, blob, expected)
}

fn validate_directory(bytes: &[u8]) -> LibraryResult<u64> {
    let footer = bytes
        .get(
            bytes
                .len()
                .checked_sub(22)
                .ok_or_else(|| invalid("missing ZIP footer"))?..,
        )
        .ok_or_else(|| invalid("missing ZIP footer"))?;
    if &footer[..4] != b"PK\x05\x06" || footer[4..8] != [0; 4] || footer[20..22] != [0; 2] {
        return Err(invalid("noncanonical ZIP footer"));
    }
    let count = u16::from_le_bytes([footer[10], footer[11]]);
    if count == 0 || count > 3 || footer[8..10] != footer[10..12] {
        return Err(invalid("unexpected entry count"));
    }
    let size = u32::from_le_bytes(footer[12..16].try_into().unwrap()) as u64;
    let start = u32::from_le_bytes(footer[16..20].try_into().unwrap()) as u64;
    if start.checked_add(size) != Some((bytes.len() - 22) as u64) {
        return Err(invalid("invalid central directory extent"));
    }
    Ok(start)
}

fn validate_snapshot(snapshot: &ResolvedSnapshot) -> LibraryResult<()> {
    let pkg = crate::semantic::parse(&snapshot.manifest, Path::new(MANIFEST))?;
    package::validate_identity(&pkg)?;
    if pkg != snapshot.package
        || !pkg.identity.revision.starts_with(crate::semantic::PREFIX)
        || crate::semantic::revision(&pkg, snapshot.document.as_deref())? != pkg.identity.revision
    {
        return Err(invalid("snapshot semantic revision mismatch"));
    }
    let valid = match (&pkg.source, &snapshot.source) {
        (
            SourceSpec::Generator {
                generator,
                generator_version,
            },
            ResolvedSource::Generator {
                generator_id,
                generator_version: version,
            },
        ) => {
            snapshot.document.is_none() && generator == generator_id && generator_version == version
        }
        (
            SourceSpec::Embedded {
                blob,
                codec,
                format,
            },
            ResolvedSource::Embedded { blob: captured },
        ) => snapshot.document.is_none() && blob_matches(blob, codec, *format, captured),
        (
            SourceSpec::Profile {
                blob,
                codec,
                format,
            },
            ResolvedSource::Profile { blob: captured },
        ) => {
            snapshot.document.is_none()
                && codec == package::PROFILE_CODEC
                && blob_matches(blob, codec, *format, captured)
        }
        (
            SourceSpec::Document {
                file,
                geometry,
                geometry_codec,
                geometry_format,
            },
            ResolvedSource::Document {
                document_sha256,
                blob,
            },
        ) => {
            Path::new(file).components().count() == 1
                && matches!(
                    Path::new(file).components().next(),
                    Some(Component::Normal(_))
                )
                && snapshot
                    .document
                    .as_ref()
                    .is_some_and(|bytes| crate::blob::sha256_hex(bytes) == *document_sha256)
                && blob_matches(geometry, geometry_codec, *geometry_format, blob)
        }
        _ => false,
    };
    if !valid {
        return Err(invalid("snapshot payload closure mismatch"));
    }
    Ok(())
}

fn blob_matches(key: &str, codec: &str, format: Option<u32>, blob: &ResolvedBlob) -> bool {
    let key = key.strip_prefix("sha256:").unwrap_or(key);
    ["step", "brep", "xbf"].contains(&codec)
        && (codec == "step" || format.is_some())
        && blob.codec == codec
        && blob.format == format
        && blob.sha256 == key
        && crate::blob::sha256_hex(&blob.bytes) == key
}

fn source_bytes(source: &ResolvedSource) -> Option<&[u8]> {
    match source {
        ResolvedSource::Generator { .. } => None,
        ResolvedSource::Embedded { blob }
        | ResolvedSource::Profile { blob }
        | ResolvedSource::Document { blob, .. } => Some(&blob.bytes),
    }
}

fn verify(
    manifest: &str,
    document: Option<Vec<u8>>,
    blob: Option<Vec<u8>>,
    expected: &ResolveRequest<'_>,
) -> LibraryResult<ResolvedSnapshot> {
    let pkg = crate::semantic::parse(manifest, Path::new(MANIFEST))?;
    package::validate_identity(&pkg)?;
    if pkg.identity.id != expected.component_id
        || pkg.identity.version != expected.component_version
        || pkg.identity.revision != expected.component_revision
        || !pkg.identity.revision.starts_with(crate::semantic::PREFIX)
        || crate::semantic::revision(&pkg, document.as_deref())? != pkg.identity.revision
    {
        return Err(invalid("component identity or semantic revision mismatch"));
    }
    let source = match &pkg.source {
        SourceSpec::Generator {
            generator,
            generator_version,
        } => {
            if document.is_some() || blob.is_some() {
                return Err(invalid("unexpected payload"));
            }
            ResolvedSource::Generator {
                generator_id: generator.clone(),
                generator_version: *generator_version,
            }
        }
        SourceSpec::Embedded {
            blob: key,
            codec,
            format,
        } => {
            if document.is_some() {
                return Err(invalid("unexpected document"));
            }
            ResolvedSource::Embedded {
                blob: verify_blob(key, codec, *format, blob)?,
            }
        }
        SourceSpec::Profile {
            blob: key,
            codec,
            format,
        } => {
            if document.is_some() || codec != package::PROFILE_CODEC {
                return Err(invalid("invalid profile closure"));
            }
            ResolvedSource::Profile {
                blob: verify_blob(key, codec, *format, blob)?,
            }
        }
        SourceSpec::Document {
            file,
            geometry,
            geometry_codec,
            geometry_format,
        } => {
            if Path::new(file).components().count() != 1
                || !matches!(
                    Path::new(file).components().next(),
                    Some(Component::Normal(_))
                )
            {
                return Err(invalid("unsafe document filename"));
            }
            let bytes = document
                .as_ref()
                .ok_or_else(|| invalid("missing document"))?;
            ResolvedSource::Document {
                document_sha256: crate::blob::sha256_hex(bytes),
                blob: verify_blob(geometry, geometry_codec, *geometry_format, blob)?,
            }
        }
    };
    Ok(ResolvedSnapshot {
        manifest: manifest.to_owned(),
        package: pkg,
        source,
        document,
    })
}

fn verify_blob(
    key: &str,
    codec: &str,
    format: Option<u32>,
    bytes: Option<Vec<u8>>,
) -> LibraryResult<ResolvedBlob> {
    if !["step", "brep", "xbf"].contains(&codec) || (codec != "step" && format.is_none()) {
        return Err(invalid("invalid blob codec or format"));
    }
    let bytes = bytes.ok_or_else(|| invalid("missing geometry"))?;
    let sha = key.strip_prefix("sha256:").unwrap_or(key);
    if crate::blob::sha256_hex(&bytes) != sha {
        return Err(invalid("geometry hash mismatch"));
    }
    Ok(ResolvedBlob {
        sha256: sha.into(),
        codec: codec.into(),
        format,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(root: &Path) -> ResolvedSnapshot {
        let dir = root.join("example.part");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("source.onecad"), b"frozen source").unwrap();
        let sha = crate::blob::BlobStore::new(root)
            .put(b"geometry payload")
            .unwrap();
        let raw = format!(
            "[identity]\nid = \"example.part\"\nversion = \"1.0.0\"\nrevision = \"sha256:{}\"\n\
             [metadata]\nname = \"Example\"\n\
             [source]\nkind = \"document\"\nfile = \"source.onecad\"\ngeometry = \"sha256:{sha}\"\ngeometry_codec = \"brep\"\ngeometry_format = 4\n",
            "0".repeat(64)
        );
        std::fs::write(dir.join(MANIFEST), &raw).unwrap();
        let pkg = package::parse(&raw, &dir).unwrap();
        let revision = package::write_package(&dir, &pkg).unwrap();
        let (index, _) = crate::index::LibraryIndex::reindex(root).unwrap();
        crate::resolve::resolve_snapshot(
            root,
            &index,
            &ResolveRequest {
                component_id: "example.part",
                component_version: "1.0.0",
                component_revision: &revision,
            },
        )
        .unwrap()
    }

    fn request(snapshot: &ResolvedSnapshot) -> ResolveRequest<'_> {
        ResolveRequest {
            component_id: &snapshot.package.identity.id,
            component_version: &snapshot.package.identity.version,
            component_revision: &snapshot.package.identity.revision,
        }
    }

    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn deterministic_round_trip_uses_only_captured_closure() {
        let dir = tempfile::tempdir().unwrap();
        let snapshot = fixture(dir.path());
        std::fs::remove_dir_all(dir.path().join("example.part")).unwrap();
        std::fs::remove_dir_all(dir.path().join("blobs")).unwrap();
        let first = encode(Some(&snapshot), 1 << 20).unwrap().unwrap();
        let second = encode(Some(&snapshot), 1 << 20).unwrap().unwrap();
        assert_eq!(first, second);
        let mut storage = vec![0; first.len() + 4096];
        let writer = encode_into(
            &snapshot,
            storage.len() as u64,
            BoundedWriter::new(&mut storage),
        )
        .unwrap();
        let used = writer.len();
        assert_eq!(&storage[..used], first.as_slice());
        let mut short = vec![0; first.len() - 1];
        assert!(encode_into(
            &snapshot,
            first.len() as u64,
            BoundedWriter::new(&mut short)
        )
        .is_err());
        let hash = crate::blob::sha256_hex(&first);
        assert_eq!(hash, crate::blob::sha256_hex(&second));
        let decoded = decode(&first, &hash, &request(&snapshot), 1 << 20).unwrap();
        assert_eq!(decoded.manifest, snapshot.manifest);
        assert_eq!(decoded.package, snapshot.package);
        assert_eq!(decoded.document, snapshot.document);
        assert_eq!(decoded.source, snapshot.source);
        assert!(encode(None, 1 << 20).unwrap().is_none());
    }

    #[test]
    fn generator_embedded_and_profile_closures_round_trip() {
        for kind in ["generator", "embedded", "profile"] {
            let dir = tempfile::tempdir().unwrap();
            let package_dir = dir.path().join("example.part");
            std::fs::create_dir_all(&package_dir).unwrap();
            let sha = crate::blob::BlobStore::new(dir.path())
                .put(b"geometry")
                .unwrap();
            let source = if kind == "generator" {
                "kind = \"generator\"\ngenerator = \"example\"\ngenerator_version = 1".to_owned()
            } else {
                format!("kind = \"{kind}\"\nblob = \"sha256:{sha}\"\ncodec = \"brep\"\nformat = 4")
            };
            let raw = format!("[identity]\nid = \"example.part\"\nversion = \"1.0.0\"\nrevision = \"sha256:{}\"\n[metadata]\nname = \"Part\"\n[source]\n{source}\n", "0".repeat(64));
            std::fs::write(package_dir.join(MANIFEST), &raw).unwrap();
            let pkg = package::parse(&raw, &package_dir).unwrap();
            let revision = package::write_package(&package_dir, &pkg).unwrap();
            let (index, _) = crate::index::LibraryIndex::reindex(dir.path()).unwrap();
            let snapshot = crate::resolve::resolve_snapshot(
                dir.path(),
                &index,
                &ResolveRequest {
                    component_id: "example.part",
                    component_version: "1.0.0",
                    component_revision: &revision,
                },
            )
            .unwrap();
            let archive = encode(Some(&snapshot), 1 << 20).unwrap().unwrap();
            let decoded = decode(
                &archive,
                &crate::blob::sha256_hex(&archive),
                &request(&snapshot),
                1 << 20,
            )
            .unwrap();
            assert_eq!(decoded.source, snapshot.source);
            assert_eq!(decoded.document, None);
        }
    }

    #[test]
    fn refuses_tampering_extra_missing_duplicate_and_oversize() {
        let dir = tempfile::tempdir().unwrap();
        let snapshot = fixture(dir.path());
        let doc = snapshot.document.as_deref().unwrap();
        let blob = source_bytes(&snapshot.source).unwrap();
        let raw = snapshot.manifest.as_bytes();
        let valid = encode(Some(&snapshot), 1 << 20).unwrap().unwrap();
        let original_hash = crate::blob::sha256_hex(&valid);
        let mut corrupt = valid.clone();
        corrupt[40] ^= 1;
        assert!(decode(&corrupt, &original_hash, &request(&snapshot), 1 << 20).is_err());
        for archive in [
            make_zip(&[(MANIFEST, raw), (DOCUMENT, doc), (BLOB, b"tampered")]),
            make_zip(&[(MANIFEST, raw), (DOCUMENT, b"tampered"), (BLOB, blob)]),
            make_zip(&[(MANIFEST, raw), (DOCUMENT, doc)]),
            make_zip(&[
                (MANIFEST, raw),
                (DOCUMENT, doc),
                (BLOB, blob),
                ("extra", b"x"),
            ]),
        ] {
            assert!(decode(
                &archive,
                &crate::blob::sha256_hex(&archive),
                &request(&snapshot),
                1 << 20
            )
            .is_err());
        }
        let mut trailing = valid.clone();
        trailing.extend_from_slice(b"unexpected trailing data");
        assert!(decode(
            &trailing,
            &crate::blob::sha256_hex(&trailing),
            &request(&snapshot),
            1 << 20
        )
        .is_err());
        let mut duplicate = valid.clone();
        let positions: Vec<_> = duplicate
            .windows(BLOB.len())
            .enumerate()
            .filter_map(|(index, part)| (part == BLOB.as_bytes()).then_some(index))
            .collect();
        assert_eq!(positions.len(), 2);
        for index in positions {
            duplicate[index..index + DOCUMENT.len()].copy_from_slice(DOCUMENT.as_bytes());
        }
        assert!(decode(
            &duplicate,
            &crate::blob::sha256_hex(&duplicate),
            &request(&snapshot),
            1 << 20
        )
        .is_err());
        assert!(decode(
            &valid,
            &original_hash,
            &request(&snapshot),
            valid.len() as u64 - 1
        )
        .is_err());
        assert!(encode(Some(&snapshot), 8).is_err());
        let wrong = ResolveRequest {
            component_id: "example.other",
            ..request(&snapshot)
        };
        assert!(decode(&valid, &original_hash, &wrong, 1 << 20).is_err());
        let mut changed = snapshot.clone();
        changed.package.metadata.name.push('!');
        assert!(encode(Some(&changed), 1 << 20).is_err());
    }
}
