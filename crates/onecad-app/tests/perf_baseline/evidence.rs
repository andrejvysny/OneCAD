use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path, process::Command};

pub(super) type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

pub(super) fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn command(program: &str, args: &[&str]) -> Value {
    match Command::new(program).args(args).output() {
        Ok(output) => {
            json!({"program":program,"args":args,"exitCode":output.status.code(),"available":output.status.success(),
            "stdout":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr)})
        }
        Err(error) => json!({"program":program,"args":args,"unavailable":error.to_string()}),
    }
}

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
}

fn git(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root())
        .output()?;
    if !output.status.success() {
        return Err(format!("source evidence failed: git {args:?}").into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

pub(super) fn source() -> Result<Value> {
    let mut untracked = BTreeMap::new();
    for name in git(&["ls-files", "--others", "--exclude-standard", "-z"])?
        .split('\0')
        .filter(|name| !name.is_empty())
    {
        untracked.insert(name.to_owned(), hash(&fs::read(root().join(name))?));
    }
    Ok(json!({"commit":git(&["rev-parse","HEAD"])?.trim(),
        "tree":git(&["rev-parse","HEAD^{tree}"])?.trim(),
        "status":git(&["status","--porcelain=v1","--untracked-files=all"])?,
        "trackedDiffSha256":hash(git(&["diff","HEAD","--binary"])?.as_bytes()),
        "untrackedSha256":untracked}))
}

pub(super) fn conditions() -> Value {
    json!({"os":command("sw_vers", &[]),"architecture":command("uname", &["-m"]),
        "hardware":command("sysctl", &["hw.model","hw.memsize"]),
        "battery":command("pmset", &["-g","batt"]),"powerSettings":command("pmset", &["-g","custom"]),
        "display":command("system_profiler", &["SPDisplaysDataType","-json"]),
        "rust":command("rustc", &["-Vv"]),"sdk":command("xcrun", &["--show-sdk-version"]),
        "settingsChangedByHarness":false,"classification":"observed conditions; no power, display or OS setting changed"})
}

pub(super) fn build_environment() -> BTreeMap<String, String> {
    std::env::vars()
        .filter(|(name, _)| {
            matches!(
                name.as_str(),
                "RUSTFLAGS" | "CARGO_ENCODED_RUSTFLAGS" | "CARGO_BUILD_TARGET"
            ) || name.starts_with("CARGO_PROFILE_RELEASE_")
        })
        .collect()
}

pub(super) fn rss(worker: &Path) -> Value {
    let output = match Command::new("ps")
        .args(["-axo", "pid=,ppid=,rss=,comm="])
        .output()
    {
        Ok(output) if output.status.success() => output,
        Ok(output) => return json!({"unavailable":format!("ps status {:?}",output.status.code())}),
        Err(error) => return json!({"unavailable":error.to_string()}),
    };
    parse_rss(
        &String::from_utf8_lossy(&output.stdout),
        std::process::id(),
        worker,
    )
}

fn parse_rss(text: &str, app: u32, worker: &Path) -> Value {
    let mut app_bytes = None;
    let mut workers = Vec::new();
    for line in text.lines() {
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }
        let (Ok(pid), Ok(parent), Ok(kib)) = (
            parts[0].parse::<u32>(),
            parts[1].parse::<u32>(),
            parts[2].parse::<u64>(),
        ) else {
            continue;
        };
        let Some(bytes) = kib.checked_mul(1024) else {
            continue;
        };
        if pid == app {
            app_bytes = Some(bytes);
        }
        let executable = parts[3..].join(" ");
        if parent == app && Path::new(&executable).file_name() == worker.file_name() {
            workers.push(json!({"pid":pid,"rssBytes":bytes}));
        }
    }
    json!({"appPid":app,"appRssBytes":app_bytes,"appUnavailable":if app_bytes.is_none() {Some("current process RSS missing from ps output")} else {None},
        "worker":if workers.len()==1 { workers[0].clone() } else { json!({"unavailable":"expected exactly one matching direct worker child","matches":workers.len()}) },
        "classification":"boundary RSS snapshots from ps; neither peak nor GPU memory"})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rss_requires_exact_child_and_preserves_units() {
        let text =
            "10 1 100 /path/app\n11 10 200 /path/onecad-worker\n12 1 300 /other/onecad-worker\n";
        let result = parse_rss(text, 10, Path::new("/path/onecad-worker"));
        assert_eq!(result["appRssBytes"], 102400);
        assert_eq!(result["worker"]["rssBytes"], 204800);
        let duplicate = format!("{text}13 10 400 /path/onecad-worker\n");
        assert!(
            parse_rss(&duplicate, 10, Path::new("onecad-worker"))["worker"]["unavailable"]
                .is_string()
        );
        assert!(parse_rss("", 10, Path::new("onecad-worker"))["appRssBytes"].is_null());
    }
}
