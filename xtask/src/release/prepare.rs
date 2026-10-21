use super::{materials::*, verify, Result};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

type Options = BTreeMap<String, PathBuf>;
const OPTIONS: &[&str] = &[
    "--source",
    "--worker",
    "--worker-manifest",
    "--occt-prefix",
    "--occt-archive",
    "--eigen-prefix",
    "--boost-prefix",
    "--boost-license",
    "--output",
];

pub fn run(args: &[String]) -> Result<()> {
    let archive = prepare(&options(args, OPTIONS)?)?;
    println!(
        "Prepared local materials: {}\nBuild, relink and publication remain unverified.",
        archive.display()
    );
    Ok(())
}

#[derive(PartialEq)]
struct State {
    revision: String,
    status: Vec<u8>,
    patch: Vec<u8>,
    names: Vec<String>,
}

fn source_state(root: &Path) -> Result<State> {
    if String::from_utf8(git(root, &["ls-files", "--stage"])?)?
        .lines()
        .any(|line| line.starts_with("160000 "))
    {
        return Err("submodules require explicit source capture support".into());
    }
    let listing = String::from_utf8(git(
        root,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )?)?;
    let mut names = BTreeSet::new();
    for name in listing.split('\0').filter(|n| !n.is_empty()) {
        safe_path(name)?;
        match fs::symlink_metadata(root.join(name)) {
            Ok(_) => {
                names.insert(name.to_owned());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(State {
        revision: String::from_utf8(git(root, &["rev-parse", "HEAD"])?)?
            .trim()
            .into(),
        status: git(
            root,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )?,
        patch: git(root, &["diff", "--binary", "HEAD", "--"])?,
        names: names.into_iter().collect(),
    })
}

fn snapshot(root: &Path, destination: &Path, names: &[String]) -> Result<()> {
    for name in names {
        let input = root.join(name);
        if !fs::symlink_metadata(&input)?.is_file() {
            return Err(format!("unsupported source type (including symlink): {name}").into());
        }
        if fs::read(&input)?.starts_with(b"version https://git-lfs.github.com/spec/v1") {
            return Err(format!("unexpanded Git LFS source: {name}").into());
        }
        let output = destination.join(name);
        fs::create_dir_all(output.parent().ok_or("missing snapshot parent")?)?;
        fs::copy(input, output)?;
    }
    check_source(destination)
}

fn assert_stable(root: &Path, snapshot: &Path, before: &State) -> Result<()> {
    if before != &source_state(root)? {
        return Err("source state changed during preparation".into());
    }
    for name in &before.names {
        if hash(&fs::read(root.join(name))?) != hash(&fs::read(snapshot.join(name))?)
            || executable(&root.join(name))? != executable(&snapshot.join(name))?
        {
            return Err(format!("source changed during preparation: {name}").into());
        }
    }
    Ok(())
}

pub(super) fn prepare(options: &Options) -> Result<PathBuf> {
    let root = canonical(&options["--source"])?;
    let output = canonical(&options["--output"])?;
    check_output(&root, &output)?;
    let (manifest_bytes, worker_hash, occt) = check_inputs(&root, options)?;
    let before = source_state(&root)?;
    let parent = output.parent().ok_or("missing output parent")?;
    fs::create_dir_all(parent)?;
    let temporary = tempfile::Builder::new()
        .prefix(".onecad-materials-")
        .tempdir_in(parent)?;
    let materials = temporary.path().join("materials");
    snapshot(&root, &materials.join("source"), &before.names)?;
    copy_external(&materials, options)?;
    let dependencies = dependencies(&materials, options)?;
    save(&materials.join("source-status.z"), &before.status)?;
    save(&materials.join("source-changes.patch"), &before.patch)?;
    save(
        &materials.join("RELINK.md"),
        relink(&occt.build_id).as_bytes(),
    )?;
    let metadata = metadata(
        &root,
        &materials,
        &before,
        worker_hash,
        &manifest_bytes,
        occt,
        dependencies,
    )?;
    save_json(&materials.join("release-materials.json"), &metadata)?;
    assert_stable(&root, &materials.join("source"), &before)?;
    finish(
        temporary,
        &output,
        &materials,
        &root,
        &before,
        &manifest_bytes,
        options,
    )
}

fn check_output(root: &Path, output: &Path) -> Result<()> {
    if String::from_utf8(git(root, &["rev-parse", "--show-toplevel"])?)?.trim()
        != root.to_str().ok_or("non-UTF-8 source root")?
    {
        return Err("--source must be repository root".into());
    }
    if output.starts_with(root) {
        return Err("--output must be outside source repository".into());
    }
    if output.exists() {
        return Err("--output already exists; refusing to overwrite".into());
    }
    Ok(())
}

fn check_inputs(root: &Path, options: &Options) -> Result<(Vec<u8>, String, Occt)> {
    let manifest_bytes = required(&options["--worker-manifest"])?;
    let manifest = worker_manifest(&manifest_bytes)?;
    let worker_hash = hash(&required(&options["--worker"])?);
    if manifest["binarySha256"] != worker_hash {
        return Err("worker SHA-256 differs from worker manifest".into());
    }
    let occt: Occt = serde_json::from_slice(&required(
        &options["--occt-prefix"].join("share/onecad/occt-build.json"),
    )?)?;
    check_occt(root, &occt, &options["--occt-archive"])?;
    check_cmake(
        &occt,
        &options["--occt-prefix"].join("share/onecad/occt-build.cmake"),
    )?;
    Ok((manifest_bytes, worker_hash, occt))
}

fn finish(
    temporary: tempfile::TempDir,
    output: &Path,
    materials: &Path,
    root: &Path,
    before: &State,
    manifest: &[u8],
    options: &Options,
) -> Result<PathBuf> {
    save_inventory(materials)?;
    let archive = temporary.path().join("onecad-source-materials.tar.gz");
    execute(
        Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(temporary.path())
            .arg("materials"),
    )?;
    verify::archive(&archive, &options["--worker"])?;
    if hash(&required(&options["--worker-manifest"])?) != hash(manifest) {
        return Err("worker manifest changed during preparation".into());
    }
    assert_stable(root, &materials.join("source"), before)?;
    save(
        &temporary.path().join("SHA256SUMS"),
        format!(
            "{}  onecad-source-materials.tar.gz\n",
            hash(&fs::read(archive)?)
        )
        .as_bytes(),
    )?;
    fs::rename(temporary.path(), output)?;
    Ok(output.join("onecad-source-materials.tar.gz"))
}

fn save_inventory(materials: &Path) -> Result<()> {
    let entries = files(materials)?
        .into_iter()
        .map(|path| {
            let bytes = fs::read(materials.join(&path))?;
            Ok(Entry {
                sha256: hash(&bytes),
                bytes: bytes.len() as u64,
                executable: executable(&materials.join(&path))?,
                path,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    save_json(
        &materials.join("inventory.json"),
        &Inventory {
            schema_version: 1,
            files: entries,
        },
    )?;
    Ok(())
}

fn metadata(
    root: &Path,
    materials: &Path,
    before: &State,
    worker_hash: String,
    manifest: &[u8],
    occt: Occt,
    dependencies: Dependencies,
) -> Result<Metadata> {
    let prepared_at = String::from_utf8(execute(
        Command::new("date").args(["-u", "+%Y-%m-%dT%H:%M:%SZ"]),
    )?)?
    .trim()
    .to_owned();
    let exact_tags = String::from_utf8(git(root, &["tag", "--points-at", "HEAD"])?)?
        .lines()
        .map(str::to_owned)
        .collect();
    let metadata = Metadata {
        schema_version: 1,
        prepared_at,
        scope: "local-source-material-preparation".into(),
        source: Source {
            revision: before.revision.clone(),
            dirty: !before.status.is_empty(),
            exact_tags,
            snapshot_policy: "tracked-and-untracked-nonignored".into(),
            file_count: files(&materials.join("source"))?.len(),
        },
        worker: Worker {
            binary_sha256: worker_hash,
            manifest_sha256: hash(manifest),
        },
        occt,
        dependencies,
        validation: Validation {
            build_provenance: "unverified".into(),
            relink: "unverified".into(),
            publication: "unverified".into(),
            transitive_license_inventory: "incomplete".into(),
        },
    };
    metadata.validate()?;
    Ok(metadata)
}

fn copy(root: &Path, name: &str, source: &Path) -> Result<()> {
    save(&root.join(name), &required(source)?)
}

fn copy_external(root: &Path, options: &Options) -> Result<()> {
    copy(root, "worker-manifest.json", &options["--worker-manifest"])?;
    for name in ["occt-build.json", "occt-build.cmake"] {
        copy(
            root,
            &format!("occt/{name}"),
            &options["--occt-prefix"].join("share/onecad").join(name),
        )?;
    }
    copy(root, "occt/OCCT-V8_0_1.tar.gz", &options["--occt-archive"])?;
    for (name, source) in [
        ("OCCT-LICENSE_LGPL_21.txt", "LICENSE_LGPL_21.txt"),
        ("OCCT-LGPL_EXCEPTION.txt", "OCCT_LGPL_EXCEPTION.txt"),
    ] {
        copy(
            root,
            &format!("licenses/{name}"),
            &options["--occt-prefix"]
                .join("share/doc/opencascade")
                .join(source),
        )?;
    }
    Ok(())
}

fn dependencies(root: &Path, options: &Options) -> Result<Dependencies> {
    let eigen_file = options["--eigen-prefix"].join("share/eigen3/cmake/Eigen3ConfigVersion.cmake");
    let boost_file = options["--boost-prefix"].join("include/boost/version.hpp");
    let eigen_text = String::from_utf8(required(&eigen_file)?)?;
    let boost_text = String::from_utf8(required(&boost_file)?)?;
    let eigen = regex::Regex::new(r#"set\(PACKAGE_VERSION "([0-9.]+)"\)"#)?
        .captures(&eigen_text)
        .and_then(|m| m.get(1))
        .ok_or("missing Eigen or Boost version evidence")?
        .as_str()
        .to_owned();
    let number: u64 = regex::Regex::new(r"(?m)^#define BOOST_VERSION\s+(\d+)$")?
        .captures(&boost_text)
        .and_then(|m| m.get(1))
        .ok_or("missing Eigen or Boost version evidence")?
        .as_str()
        .parse()?;
    let boost = format!(
        "{}.{}.{}",
        number / 100000,
        (number / 100) % 1000,
        number % 100
    );
    copy(root, "dependencies/eigen-version.cmake", &eigen_file)?;
    copy(root, "dependencies/boost-version.hpp", &boost_file)?;
    for name in ["README", "MPL2", "BSD", "APACHE", "MINPACK"] {
        copy(
            root,
            &format!("licenses/Eigen-COPYING.{name}"),
            &options["--eigen-prefix"].join(format!("COPYING.{name}")),
        )?;
    }
    copy(
        root,
        "licenses/Boost-LICENSE_1_0.txt",
        &options["--boost-license"],
    )?;
    Ok(Dependencies { eigen, boost })
}

fn relink(build_id: &str) -> String {
    include_str!("relink.md").replace("@BUILD_ID@", build_id)
}
