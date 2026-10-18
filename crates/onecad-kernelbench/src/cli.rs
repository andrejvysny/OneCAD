use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::campaign;
use crate::case::Case;
use crate::case_v2::CaseV2;
use crate::prepared::PreparedCase;
use crate::report;
use crate::result::Backend;
use crate::runner;
use crate::search;
use crate::suite::{self, Variant, VariantName};
use crate::suite_v2;
use crate::{AppError, AppResult};

pub fn run(args: Vec<String>) -> AppResult<i32> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(usage());
    };
    match command {
        "run" => run_suite(parse(&args[1..])?),
        "run-case" => run_case(parse(&args[1..])?),
        "search-critical" => search_critical(parse(&args[1..])?),
        "report" => run_report(parse(&args[1..])?),
        "-h" | "--help" | "help" => {
            print_usage();
            Ok(0)
        }
        _ => Err(usage()),
    }
}

#[derive(Debug)]
struct Arguments {
    positionals: Vec<String>,
    flags: BTreeMap<String, String>,
}

fn parse(args: &[String]) -> AppResult<Arguments> {
    let mut parsed = Arguments {
        positionals: Vec::new(),
        flags: BTreeMap::new(),
    };
    let mut index = 0;
    while index < args.len() {
        if args[index].starts_with("--") {
            let value = args
                .get(index + 1)
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(usage)?;
            if parsed
                .flags
                .insert(args[index].clone(), value.clone())
                .is_some()
            {
                return Err(AppError::cli("duplicate option"));
            }
            index += 2;
        } else {
            parsed.positionals.push(args[index].clone());
            index += 1;
        }
    }
    Ok(parsed)
}

fn run_suite(args: Arguments) -> AppResult<i32> {
    require_no_positionals(&args)?;
    require_flags(
        &args,
        &["--suite", "--preset", "--backend", "--out-dir"],
        &["--runner"],
    )?;
    let cases = match (flag(&args, "--suite")?, flag(&args, "--preset")?) {
        ("fillet/foundation", "t0") => suite::t0(),
        ("fillet/matrix", "m1") => suite_v2::m1(),
        _ => return Err(AppError::cli("unsupported suite or preset")),
    };
    let backends = parse_backend(flag(&args, "--backend")?)?;
    let runner = runner::resolve_runner(optional_path(&args, "--runner"))?;
    campaign::run_cases(
        &runner,
        &cases,
        &backends,
        &PathBuf::from(flag(&args, "--out-dir")?),
    )
}

/// A case file names its own format. Dispatching on `schemaVersion` is what
/// keeps a v2 document from being read through v1's rules, which would
/// benchmark different intent than the file describes.
fn load_case(path: &PathBuf) -> AppResult<PreparedCase> {
    let bytes = std::fs::read(path).map_err(|e| AppError::cli(format!("case: {e}")))?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| AppError::cli(format!("case: {e}")))?;
    match value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
    {
        Some(1) => Ok(Case::load(path)?.prepared()),
        Some(2) => Ok(CaseV2::load(path)?.prepared()),
        _ => Err(AppError::cli("case schemaVersion must be 1 or 2")),
    }
}

fn run_case(args: Arguments) -> AppResult<i32> {
    require_positionals(&args, 1)?;
    require_flags(&args, &["--backend", "--out-dir"], &["--runner"])?;
    let case = load_case(&PathBuf::from(&args.positionals[0]))?;
    let backends = parse_backend(flag(&args, "--backend")?)?;
    let runner = runner::resolve_runner(optional_path(&args, "--runner"))?;
    let variant = Variant {
        name: VariantName::Base,
        translation: None,
        rotation: None,
        mirror: None,
        scale: None,
        parameter_epsilon: None,
        edge_order_permutation: None,
        contour_seed: None,
    };
    campaign::run_one(
        &runner,
        case,
        variant,
        &backends,
        &PathBuf::from(flag(&args, "--out-dir")?),
    )
}

fn search_critical(args: Arguments) -> AppResult<i32> {
    require_positionals(&args, 1)?;
    require_flags(&args, &["--param", "--backend", "--out-dir"], &["--runner"])?;
    if flag(&args, "--param")? != "operation.radius" {
        return Err(AppError::cli("only operation.radius is searchable"));
    }
    let case = Case::load(&PathBuf::from(&args.positionals[0]))?;
    let backends = parse_backend(flag(&args, "--backend")?)?;
    let runner = runner::resolve_runner(optional_path(&args, "--runner"))?;
    search::run_search(
        &runner,
        case,
        &backends,
        &PathBuf::from(flag(&args, "--out-dir")?),
    )
}

fn run_report(args: Arguments) -> AppResult<i32> {
    require_positionals(&args, 1)?;
    require_flags(&args, &["--json"], &[])?;
    report::report(
        &PathBuf::from(&args.positionals[0]),
        &PathBuf::from(flag(&args, "--json")?),
    )
}

fn parse_backend(value: &str) -> AppResult<Vec<Backend>> {
    match value {
        "raw-occt" => Ok(vec![Backend::RawOcct]),
        "onecad" => Ok(vec![Backend::Onecad]),
        "both" => Ok(vec![Backend::RawOcct, Backend::Onecad]),
        _ => Err(AppError::cli("backend must be raw-occt, onecad, or both")),
    }
}

fn require_flags(args: &Arguments, required: &[&str], optional: &[&str]) -> AppResult<()> {
    if required.iter().any(|key| !args.flags.contains_key(*key)) {
        return Err(usage());
    }
    if args
        .flags
        .keys()
        .any(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
    {
        return Err(AppError::cli("unknown option"));
    }
    Ok(())
}

fn require_positionals(args: &Arguments, count: usize) -> AppResult<()> {
    if args.positionals.len() == count {
        Ok(())
    } else {
        Err(usage())
    }
}

fn require_no_positionals(args: &Arguments) -> AppResult<()> {
    require_positionals(args, 0)
}

fn flag<'a>(args: &'a Arguments, key: &str) -> AppResult<&'a str> {
    args.flags.get(key).map(String::as_str).ok_or_else(usage)
}

fn optional_path(args: &Arguments, key: &str) -> Option<PathBuf> {
    args.flags.get(key).map(PathBuf::from)
}

fn usage() -> AppError {
    AppError::cli("invalid arguments; use --help")
}

fn print_usage() {
    eprintln!("onecad-kernelbench run --suite fillet/foundation --preset t0 | --suite fillet/matrix --preset m1 --backend raw-occt|onecad|both [--runner PATH] --out-dir DIR");
    eprintln!("onecad-kernelbench run-case CASE.json --backend raw-occt|onecad|both [--runner PATH] --out-dir DIR");
    eprintln!("onecad-kernelbench search-critical CASE.json --param operation.radius --backend raw-occt|onecad|both [--runner PATH] --out-dir DIR");
    eprintln!("onecad-kernelbench report RESULTS.jsonl --json SUMMARY.json");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_both_has_stable_order() {
        let parsed = parse_backend("both").unwrap();
        assert_eq!(parsed, vec![Backend::RawOcct, Backend::Onecad]);
    }

    #[test]
    fn unknown_option_is_rejected() {
        let args = parse(&["--bogus".into(), "x".into()]).unwrap();
        assert!(require_flags(&args, &[], &[]).is_err());
    }
}
