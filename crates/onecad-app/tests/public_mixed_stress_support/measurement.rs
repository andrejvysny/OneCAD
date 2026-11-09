//! Append-only measurement evidence. Completion never establishes qualification.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

pub struct Campaign {
    pub root: PathBuf,
    pub batches: usize,
    pub warmups: usize,
    pub warm: usize,
    pub cold: usize,
    smoke: bool,
    samples: fs::File,
    records: Vec<Value>,
    metadata: Value,
    artifacts: BTreeMap<String, PathBuf>,
}

impl Campaign {
    pub fn begin(workload: &str, fixture: &Path, notes: Value) -> Self {
        require_release();
        let smoke = std::env::var("ONECAD_PERF_SMOKE").as_deref() == Ok("1");
        let base = PathBuf::from(std::env::var_os("ONECAD_PERF_OUT").unwrap());
        let repository = repository_root().expect("repository root must resolve");
        let root = prepare_output(&base, workload, &repository)
            .expect("performance output must be new and outside repository");
        fs::copy(fixture, root.join("fixture.onecad")).unwrap();
        let config = json!({"batches":if smoke {1} else {3}, "warmups":if smoke {1} else {5},
            "warm":if smoke {2} else {100}, "cold":if smoke {1} else {20}, "smoke":smoke,
            "stderrTracing":std::env::var_os("ONECAD_PERF_TRACE").is_some()});
        let artifacts = BTreeMap::from([
            ("fixture".into(), root.join("fixture.onecad")),
            ("originalFixture".into(), fs::canonicalize(fixture).unwrap()),
            (
                "worker".into(),
                fs::canonicalize(super_worker_path()).unwrap(),
            ),
            ("testExecutable".into(), std::env::current_exe().unwrap()),
        ]);
        let digests = artifact_digests(&artifacts).expect("measurement inputs must be readable");
        assert_eq!(
            digests["fixture"], digests["originalFixture"],
            "fixture changed during evidence capture"
        );
        let metadata = json!({"schemaVersion":1,"workload":workload,"config":config,
            "configSha256":hash(&serde_json::to_vec(&config).unwrap()),
            "source":source_at(&repository).expect("source provenance must be complete"),
            "artifactDigests":digests,"artifactPaths":artifacts,
            "fixtureSha256":digests["fixture"],"workerSha256":digests["worker"],
            "conditionsStart":conditions(),"testExecutableSha256":digests["testExecutable"],
            "commandLine":std::env::args().collect::<Vec<_>>(),
            "notes":notes,"unavailable":["physical input","preview admission","bound-origin resolution", "GPU memory", "GPU completion", "presentation", "peak RSS"]});
        fs::write(
            root.join("metadata.json"),
            serde_json::to_vec_pretty(&metadata).unwrap(),
        )
        .unwrap();
        fs::write(root.join("summary.json"), b"{\"status\":\"incomplete\"}").unwrap();
        let samples = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(root.join("samples.jsonl"))
            .unwrap();
        Self {
            root,
            batches: if smoke { 1 } else { 3 },
            warmups: if smoke { 1 } else { 5 },
            warm: if smoke { 2 } else { 100 },
            cold: if smoke { 1 } else { 20 },
            smoke,
            samples,
            records: Vec::new(),
            metadata,
            artifacts,
        }
    }

    pub fn record(&mut self, mut record: Value) {
        record["recordIndex"] = json!(self.records.len());
        serde_json::to_writer(&mut self.samples, &record).unwrap();
        self.samples.write_all(b"\n").unwrap();
        self.samples.sync_all().unwrap();
        self.records.push(record);
    }

    pub fn finish(mut self) {
        let mut groups: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        for row in self.records.iter().filter(|row| row["event"] == "result") {
            if let Some(value) = row["wallMs"].as_f64() {
                let key = format!(
                    "batch={}/phase={}/case={}",
                    row["batch"], row["phase"], row["case"]
                );
                groups.entry(key).or_default().push(value);
            }
        }
        let distributions: BTreeMap<_,_> = groups.into_iter().map(|(key, mut values)| {
            values.sort_by(f64::total_cmp);
            let percentile = |p: f64| values[((values.len() as f64*p).ceil() as usize).saturating_sub(1)];
            (key,json!({"count":values.len(),"p50Ms":percentile(0.5),"p95Ms":percentile(0.95),"p99Ms":percentile(0.99)}))
        }).collect();
        let verification = repository_root().and_then(|repository| {
            assess_end(
                &self.metadata,
                source_at(&repository),
                artifact_digests(&self.artifacts),
            )
        });
        let verified = verification.is_ok();
        self.metadata["endVerification"] = match &verification {
            Ok(evidence) => evidence.clone(),
            Err(error) => json!({"verified":false,"failure":error}),
        };
        self.metadata["conditionsEnd"] = conditions();
        fs::write(
            self.root.join("metadata.json"),
            serde_json::to_vec_pretty(&self.metadata).unwrap(),
        )
        .unwrap();
        fs::write(self.root.join("summary.json"),serde_json::to_vec_pretty(&json!({
            "status":if !verified {"incomplete-evidence-changed"} else if self.smoke {"incomplete-smoke-only"} else {"measurement-complete"},
            "qualificationPassed":false,"evidenceVerified":verified,"failure":verification.as_ref().err(),
            "recordCount":self.records.len(),"distributions":distributions})).unwrap()).unwrap();
        verification.expect("measurement evidence changed or could not be verified");
    }
}

fn super_worker_path() -> PathBuf {
    std::env::var_os("ONECAD_WORKER_PATH")
        .map(PathBuf::from)
        .expect("real worker required")
}

pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn command(program: &str, args: &[&str]) -> Value {
    match Command::new(program).args(args).output() {
        Ok(output) => {
            json!({"status":output.status.code(),"stdout":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr)})
        }
        Err(error) => json!({"unavailable":error.to_string()}),
    }
}

type EvidenceResult<T> = Result<T, String>;

fn repository_root() -> EvidenceResult<PathBuf> {
    fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .map_err(|error| format!("repository root: {error}"))
}

fn git_at(repository: &Path, args: &[&str]) -> EvidenceResult<String> {
    let output = Command::new("git")
        .current_dir(repository)
        .args(args)
        .output()
        .map_err(|error| format!("git {args:?}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| format!("git {args:?}: {error}"))
}

fn source_at(repository: &Path) -> EvidenceResult<Value> {
    let names = git_at(
        repository,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?;
    Ok(json!({"commit":git_at(repository,&["rev-parse","HEAD"])?,
        "tree":git_at(repository,&["rev-parse","HEAD^{tree}"])?,
        "status":git_at(repository,&["status","--porcelain=v1","--untracked-files=all"])?,
        "trackedDiffSha256":hash(git_at(repository,&["diff","HEAD","--binary"])?.as_bytes()),
        "untrackedFiles":untracked_digests(repository,&names)?}))
}

fn untracked_digests(repository: &Path, names: &str) -> EvidenceResult<BTreeMap<String, String>> {
    names
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(|path| Ok((path.to_owned(), file_digest(&repository.join(path))?)))
        .collect()
}

fn file_digest(path: &Path) -> EvidenceResult<String> {
    let mut file = fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn artifact_digests(paths: &BTreeMap<String, PathBuf>) -> EvidenceResult<BTreeMap<String, String>> {
    paths
        .iter()
        .map(|(name, path)| Ok((name.clone(), file_digest(path)?)))
        .collect()
}

fn assess_end(
    metadata: &Value,
    source: EvidenceResult<Value>,
    digests: EvidenceResult<BTreeMap<String, String>>,
) -> EvidenceResult<Value> {
    let source = source?;
    let digests = digests?;
    if source != metadata["source"] {
        return Err("source provenance changed during measurement".into());
    }
    if json!(digests) != metadata["artifactDigests"] {
        return Err("fixture, worker or test executable changed during measurement".into());
    }
    Ok(json!({"verified":true,"source":source,"artifactDigests":digests}))
}

fn prepare_output(base: &Path, workload: &str, repository: &Path) -> EvidenceResult<PathBuf> {
    let repository = fs::canonicalize(repository).map_err(|error| error.to_string())?;
    let mut components = Path::new(workload).components();
    if !matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    ) {
        return Err("workload output name must be one directory".into());
    }
    let requested = std::path::absolute(base).map_err(|error| error.to_string())?;
    let base = if requested.exists() {
        fs::canonicalize(&requested)
    } else {
        fs::canonicalize(requested.parent().ok_or("output parent required")?)
            .map(|parent| parent.join(requested.file_name().expect("absolute output has a name")))
    }
    .map_err(|error| format!("output directory: {error}"))?;
    if base.starts_with(&repository) {
        return Err("performance output must be outside repository".into());
    }
    let marker = base.join(".onecad-performance-base.json");
    let identity = b"{\"schemaVersion\":1,\"kind\":\"onecad-performance-output-base\"}";
    if base.exists() {
        if fs::read(&marker)
            .map_err(|error| format!("existing output base is not owned: {error}"))?
            != identity
        {
            return Err("existing output base is not a performance campaign base".into());
        }
    } else {
        fs::create_dir(&base).map_err(|error| error.to_string())?;
        fs::write(&marker, identity).map_err(|error| error.to_string())?;
    }
    let output = base.join(workload);
    fs::create_dir(&output).map_err(|error| format!("workload output must be new: {error}"))?;
    Ok(output)
}

fn conditions() -> Value {
    json!({"timeUtc":command("date",&["-u","+%Y-%m-%dT%H:%M:%SZ"]),"os":command("sw_vers",&[]),"hardwareAndDisplay":command("system_profiler",&["SPHardwareDataType","SPDisplaysDataType","-json"]),
        "power":command("pmset",&["-g","batt"]),"rust":command("rustc",&["-Vv"]),"architecture":std::env::consts::ARCH})
}

pub fn rss() -> Value {
    let output = Command::new("ps")
        .args(["-axo", "pid=,ppid=,rss=,comm="])
        .output();
    let output = match output {
        Ok(output) => output,
        Err(error) => return json!({"unavailable":error.to_string()}),
    };
    if !output.status.success() {
        return json!({"unavailable":"ps refused process inventory", "status":output.status.code(), "stderr":String::from_utf8_lossy(&output.stderr)});
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let pid = std::process::id();
    let rows: Vec<_> = text
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            Some((
                words.next()?.parse::<u32>().ok()?,
                words.next()?.parse::<u32>().ok()?,
                words.next()?.parse::<u64>().ok()?,
                words.collect::<Vec<_>>().join(" "),
            ))
        })
        .collect();
    let app = rows.iter().find(|row| row.0 == pid).map(|row| row.2 * 1024);
    let children: Vec<_> = rows
        .iter()
        .filter(|row| row.1 == pid && row.3.contains("onecad-worker"))
        .map(|row| json!({"pid":row.0,"rssBytes":row.2*1024,"command":row.3}))
        .collect();
    json!({"appPid":pid,"appRssBytes":app,"workerProcesses":children,"classification":"boundary RSS only; auxiliary processes may end before sampling"})
}

pub fn require_release() {
    #[cfg(debug_assertions)]
    panic!("ONECAD_PERF_OUT requires --release");
}

pub fn optional_tracing() {
    if std::env::var_os("ONECAD_PERF_TRACE").is_some() {
        tracing_subscriber::fmt()
            .with_env_filter(onecad_app::logging::env_filter())
            .with_ansi(false)
            .with_writer(std::io::stderr)
            .try_init()
            .expect("performance diagnostic tracing must be installed once");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_reads_untracked_paths_from_repository_root_and_refuses_errors() {
        let area = tempfile::tempdir().unwrap();
        let repository = area.path();
        git_at(repository, &["init", "-q"]).unwrap();
        fs::create_dir_all(repository.join("crates/example")).unwrap();
        fs::write(repository.join("root-source.rs"), b"outside Cargo crate").unwrap();
        fs::write(repository.join("crates/example/local.rs"), b"inside crate").unwrap();
        let names = git_at(
            repository,
            &["ls-files", "--others", "--exclude-standard", "-z"],
        )
        .unwrap();
        let captured = untracked_digests(repository, &names).unwrap();
        assert_eq!(captured["root-source.rs"], hash(b"outside Cargo crate"));
        assert_eq!(captured["crates/example/local.rs"], hash(b"inside crate"));
        fs::remove_file(repository.join("root-source.rs")).unwrap();
        assert!(untracked_digests(repository, &names).is_err());
        assert!(
            source_at(repository).is_err(),
            "missing HEAD must fail provenance"
        );
        assert!(source_at(&repository.join("not-a-directory")).is_err());
    }

    #[test]
    fn output_requires_outside_repository_new_workload_and_owned_shared_base() {
        let area = tempfile::tempdir().unwrap();
        let repository = area.path().join("repository");
        fs::create_dir(&repository).unwrap();
        assert!(prepare_output(&repository.join("out"), "first", &repository).is_err());
        assert!(prepare_output(area.path(), "first", &repository).is_err());
        let base = area.path().join("measurements");
        assert!(prepare_output(&base, "first", &repository).is_ok());
        assert!(prepare_output(&base, "second", &repository).is_ok());
        assert!(prepare_output(&base, "first", &repository).is_err());
        assert!(prepare_output(&base, "../escape", &repository).is_err());
        assert!(prepare_output(&base, "/", &repository).is_err());
        fs::write(base.join(".onecad-performance-base.json"), b"changed owner").unwrap();
        assert!(prepare_output(&base, "third", &repository).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&repository, area.path().join("alias")).unwrap();
            assert!(
                prepare_output(&area.path().join("alias"), "outside-name", &repository).is_err()
            );
        }
    }

    #[test]
    fn end_verification_rejects_each_changed_or_unreadable_input_and_source() {
        let area = tempfile::tempdir().unwrap();
        let paths: BTreeMap<_, _> = ["fixture", "originalFixture", "worker", "testExecutable"]
            .into_iter()
            .map(|name| {
                let path = area.path().join(name);
                fs::write(&path, b"original").unwrap();
                (name.to_owned(), path)
            })
            .collect();
        let source = json!({"commit":"same","untrackedFiles":{}});
        let metadata = json!({"source":source,"artifactDigests":artifact_digests(&paths).unwrap()});
        assert!(assess_end(&metadata, Ok(source.clone()), artifact_digests(&paths)).is_ok());
        for path in paths.values() {
            fs::write(path, b"tampered").unwrap();
            assert!(assess_end(&metadata, Ok(source.clone()), artifact_digests(&paths)).is_err());
            fs::write(path, b"original").unwrap();
        }
        assert!(assess_end(
            &metadata,
            Ok(json!({"commit":"changed"})),
            artifact_digests(&paths)
        )
        .is_err());
        assert!(assess_end(
            &metadata,
            Err("git failed".into()),
            artifact_digests(&paths)
        )
        .is_err());
        fs::remove_file(&paths["worker"]).unwrap();
        assert!(assess_end(&metadata, Ok(source), artifact_digests(&paths)).is_err());
    }
}
