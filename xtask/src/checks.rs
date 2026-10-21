//! Repository dependency and executable modeling-contract guards.
mod contracts;
mod coverage;
mod layers;
#[cfg(test)]
mod tests;

use regex::Regex;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub fn run(command: &str, args: &[String]) -> Result<()> {
    if !args.is_empty() {
        return Err(format!("{command}: unexpected arguments: {}", args.join(" ")).into());
    }
    let root = std::env::current_dir()?;
    match command {
        "check-rust-layers" => layers::run(&root),
        "verify-modeling-coverage" => coverage::run(&root),
        "verify-modeling-contracts" => contracts::run(&root),
        _ => Err(format!("unknown check: {command}").into()),
    }
}

fn read(root: &Path, path: &str) -> Result<String> {
    Ok(std::fs::read_to_string(root.join(path))?)
}

fn json(path: &Path) -> Result<Value> {
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
}

fn input(root: &Path, variable: &str, default: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(default))
}

fn string<'a>(row: &'a Value, key: &str) -> &'a str {
    row[key].as_str().unwrap_or("")
}

fn list(value: &str) -> Vec<&str> {
    value
        .split(';')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .collect()
}

fn captures(source: &str, pattern: &str) -> Result<BTreeSet<String>> {
    Ok(Regex::new(pattern)?
        .captures_iter(source)
        .map(|m| m[1].to_owned())
        .collect())
}

fn enum_variants(source: &str, name: &str, suffix: &str) -> Result<BTreeSet<String>> {
    let blocks = Regex::new(&format!(r"(?ms)^pub enum {name} \{{(.*?)^\}}"))?;
    let mut variants = BTreeSet::new();
    for block in blocks.captures_iter(source) {
        variants.extend(captures(
            &block[1],
            &format!(r"(?m)^\s{{4}}([A-Z][A-Za-z]*){suffix}"),
        )?);
    }
    Ok(variants)
}

fn finish(label: &str, errors: Vec<String>) -> Result<()> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors
            .into_iter()
            .map(|e| format!("{label}: {e}"))
            .collect::<Vec<_>>()
            .join("\n")
            .into())
    }
}
