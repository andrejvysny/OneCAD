//! Reproducible automated qualification; physical input and OS acceptance stay separate.
mod campaign;
mod inventories;
mod manifest;
mod nested;
#[cfg(test)]
mod tests;

use std::{fs, path::Path, process::Command};

pub(super) const PYTHON_SUITES: &[&str] = &[
    "fixtures/qualification/joint-publication-observer/test_check.py",
    "fixtures/reference-oracles/nist-ftc09-direct/test_circle_pairs.py",
    "fixtures/reference-oracles/nist-ftc09-direct/source_bounds/test_certificate.py",
    "fixtures/reference-oracles/nist-ftc09-direct/test_trace.py",
    "fixtures/reference-oracles/nist-ftc09-direct/analytic_mass/test_audit.py",
    "fixtures/reference-oracles/test_prusa_source_stl.py",
];

const PYTHON_RUNNER: &str = r#"
import importlib.util, pathlib, sys, unittest
path = pathlib.Path(sys.argv[1]).resolve(strict=True)
sys.path.insert(0, str(path.parent))
spec = importlib.util.spec_from_file_location('required_oracle', path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
suite = unittest.defaultTestLoader.loadTestsFromModule(module)
def names(suite):
    for test in suite:
        if isinstance(test, unittest.TestSuite):
            yield from names(test)
        else:
            yield test.id()
inventory = list(names(suite))
if not inventory or len(set(inventory)) != len(inventory):
    raise SystemExit('Required Python suite is empty or duplicated')
print('\n'.join(inventory), flush=True)
result = unittest.TextTestRunner(verbosity=2).run(suite)
if not result.wasSuccessful() or result.skipped or result.testsRun != len(inventory):
    raise SystemExit('Required Python suite failed, skipped, or incomplete')
"#;

pub fn run(command: &str, args: &[String]) -> crate::Result {
    match command {
        "python-oracles" if args.is_empty() => python_oracles(Path::new(".")),
        "qualify-macos" => campaign::run(args),
        "verify-qualification" if args.len() == 1 => manifest::verify(Path::new(&args[0])),
        _ => Err("usage: python-oracles | qualify-macos --out DIRECTORY | verify-qualification DIRECTORY".into()),
    }
}

fn python_oracles(root: &Path) -> crate::Result {
    for path in PYTHON_SUITES {
        let path = root.join(path);
        fs::metadata(&path)?;
        let status = Command::new("python3")
            .args(["-c", PYTHON_RUNNER])
            .arg(&path)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .status()?;
        if !status.success() {
            return Err(format!("required Python suite failed: {}", path.display()).into());
        }
    }
    Ok(())
}
