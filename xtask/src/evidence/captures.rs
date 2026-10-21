use super::Result;
use regex::Regex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::Path};

pub(super) fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn verify(directory: &Path) -> Result<Value> {
    let files = fs::read_dir(directory)?
        .map(|entry| {
            Ok(entry?
                .file_name()
                .into_string()
                .map_err(|_| "unsafe capture path")?)
        })
        .collect::<Result<BTreeSet<String>>>()?;
    let files: BTreeSet<_> = files
        .into_iter()
        .filter(|name| name != "manifest.json")
        .collect();
    let pngs: Vec<_> = files.iter().filter(|name| name.ends_with(".png")).collect();
    if pngs.is_empty() {
        return Err("renderer evidence contains no images".into());
    }
    let safe = Regex::new(r"^[a-zA-Z0-9_-]+\.(png|txt)$")?;
    let mut records = Vec::new();
    for name in &files {
        if !safe.is_match(name) || !fs::symlink_metadata(directory.join(name))?.is_file() {
            return Err(format!("unsafe capture path: {name}").into());
        }
        let (stem, extension) = name.rsplit_once('.').ok_or("unsafe capture path")?;
        let peer = format!("{stem}.{}", if extension == "png" { "txt" } else { "png" });
        if !files.contains(&peer) {
            return Err(format!("missing capture pair: {name}").into());
        }
        let bytes = fs::read(directory.join(name))?;
        if bytes.is_empty() {
            return Err(format!("empty capture evidence: {name}").into());
        }
        records.push(json!({"name":name,"bytes":bytes.len(),"sha256":hash(&bytes)}));
    }
    let (cases, sources) = provenance(directory, &pngs)?;
    if sources.len() != 1 {
        return Err("capture source revisions differ".into());
    }
    for depth in ["fallback", "stencil"] {
        for projection in ["perspective", "orthographic"] {
            for theme in ["dark", "light"] {
                let name = format!("{depth}-{projection}-{theme}-shaded-edges");
                if !cases.contains(&name) {
                    return Err(format!("missing renderer configuration: {name}").into());
                }
            }
        }
    }
    Ok(json!({"source":sources.first(),"images":pngs.len(),"files":records}))
}

fn provenance(directory: &Path, pngs: &[&String]) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    let mut cases = BTreeSet::new();
    let mut sources = BTreeSet::new();
    let source = Regex::new(r"(?mR)^source_commit: ([a-f0-9]{40})$")?;
    let case = Regex::new(r#"(?mR)^case: "([^"]+)"$"#)?;
    for name in pngs {
        let image = fs::read(directory.join(name))?;
        if image.len() < 33
            || image[..8] != [137, 80, 78, 71, 13, 10, 26, 10]
            || &image[12..16] != b"IHDR"
            || image[16..20] == [0; 4]
            || image[20..24] == [0; 4]
        {
            return Err(format!("invalid PNG: {name}").into());
        }
        let metadata = fs::read_to_string(directory.join(name.replace(".png", ".txt")))?;
        let revision = source.captures(&metadata);
        if revision.is_none()
            || !metadata.contains("features:")
            || !metadata.contains("backend: Metal")
            || !metadata.contains("source_status:")
        {
            return Err(format!("missing capture provenance: {name}").into());
        }
        sources.insert(revision.unwrap()[1].to_string());
        if let Some(found) = case.captures(&metadata) {
            cases.insert(found[1].to_string());
        }
    }
    Ok((cases, sources))
}
