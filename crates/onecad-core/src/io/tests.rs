use super::*;

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::{document::Document, ids::DocumentId};
use container::{ContainerCaches, ContainerReader, ContainerWriter, SaveMeta};

const CHILD_MODE: &str = "ONECAD_IO_TEST_CHILD_MODE";
const CHILD_ROOT: &str = "ONECAD_IO_TEST_CHILD_ROOT";
const CHILD_ADDRESS: &str = "ONECAD_IO_TEST_CHILD_ADDRESS";
const HELD_BYTES: &[u8] = b"another process owns these staged bytes";

struct WriterProcess(Option<Child>);

impl WriterProcess {
    fn finish(mut self) {
        let output = self.0.take().unwrap().wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "child failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

impl Drop for WriterProcess {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn spawn_writer(
    test: &str,
    mode: &str,
    root: &Path,
    address: std::net::SocketAddr,
    extra: &[(&str, String)],
) -> WriterProcess {
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture"])
        .env(CHILD_MODE, mode)
        .env(CHILD_ROOT, root)
        .env(CHILD_ADDRESS, address.to_string())
        .envs(extra.iter().map(|(key, value)| (*key, value)))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    WriterProcess(Some(child))
}

fn accept_writer(listener: &TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                return stream;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "child did not connect");
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(error) => panic!("accept child: {error}"),
        }
    }
}

fn child_connection(mode: &str) -> Option<(PathBuf, TcpStream)> {
    if std::env::var(CHILD_MODE).as_deref() != Ok(mode) {
        return None;
    }
    let root = PathBuf::from(std::env::var_os(CHILD_ROOT).unwrap());
    let stream = TcpStream::connect(std::env::var(CHILD_ADDRESS).unwrap()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    Some((root, stream))
}

fn hold_staged_temp(mode: &str) -> bool {
    let Some((root, mut stream)) = child_connection(mode) else {
        return false;
    };
    let target = root.join("model.onecad");
    let path = match mode {
        "held-json" => {
            let path = target.with_file_name("model.onecad.tmp");
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            file.write_all(HELD_BYTES).unwrap();
            file.sync_all().unwrap();
            path
        }
        "held-container" => ContainerWriter::save_leaving_temp(
            &target,
            &Document::new(DocumentId::new()),
            &ContainerCaches::none(),
            &SaveMeta::default(),
        )
        .unwrap(),
        _ => panic!("unknown staging mode"),
    };
    let _active = File::open(&path).unwrap();
    writeln!(stream, "{}", path.display()).unwrap();
    stream.read_exact(&mut [0]).unwrap();
    fs::remove_file(path).unwrap();
    true
}

fn staged_writer(test: &str, mode: &str, root: &Path) -> (WriterProcess, TcpStream, PathBuf) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let child = spawn_writer(test, mode, root, listener.local_addr().unwrap(), &[]);
    let stream = accept_writer(&listener);
    let mut path = String::new();
    BufReader::new(stream.try_clone().unwrap())
        .read_line(&mut path)
        .unwrap();
    (child, stream, PathBuf::from(path.trim_end()))
}

#[test]
fn durable_write_preserves_another_process_staged_temp() {
    if hold_staged_temp("held-json") {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let (child, mut stream, active) = staged_writer(
        "io::tests::durable_write_preserves_another_process_staged_temp",
        "held-json",
        root.path(),
    );
    let target = root.path().join("model.onecad");
    durable_write(&target, b"new complete bytes").unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"new complete bytes");
    assert_eq!(
        fs::read(&active).unwrap(),
        HELD_BYTES,
        "another process's staging must survive"
    );
    stream.write_all(b"x").unwrap();
    child.finish();
}

#[test]
fn container_save_preserves_another_process_staged_archive() {
    if hold_staged_temp("held-container") {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let (child, mut stream, active) = staged_writer(
        "io::tests::container_save_preserves_another_process_staged_archive",
        "held-container",
        root.path(),
    );
    let held = fs::read(&active).unwrap();
    let target = root.path().join("model.onecad");
    let document = Document::new(DocumentId::new());
    ContainerWriter::save(
        &target,
        &document,
        &ContainerCaches::none(),
        &SaveMeta::default(),
    )
    .unwrap();
    assert_eq!(
        ContainerReader::open(&target).unwrap().document().id,
        document.id
    );
    assert_eq!(
        fs::read(&active).unwrap(),
        held,
        "an active staged archive is not stale"
    );
    stream.write_all(b"x").unwrap();
    child.finish();
}

#[test]
fn exclusive_creation_collision_never_claims_the_existing_file() {
    let root = tempfile::tempdir().unwrap();
    let candidate = root.path().join(".settings.json.tmp-existing");
    fs::write(&candidate, HELD_BYTES).unwrap();
    let error = match OwnedTempFile::create_at(candidate.clone()) {
        Err(error) => error,
        Ok(_) => panic!("existing staging must not be opened"),
    };
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(candidate).unwrap(), HELD_BYTES);
}

fn entries(root: &Path) -> Vec<std::ffi::OsString> {
    let mut result: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    result.sort();
    result
}

#[test]
fn failed_replacement_preserves_source_data_and_another_process_temp() {
    if hold_staged_temp("held-json") || hold_staged_temp("held-container") {
        return;
    }
    for mode in ["held-json", "held-container"] {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("model.onecad");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("original"), b"existing destination data").unwrap();
        let (child, mut stream, active) = staged_writer(
            "io::tests::failed_replacement_preserves_source_data_and_another_process_temp",
            mode,
            root.path(),
        );
        let held = fs::read(&active).unwrap();
        let before = entries(root.path());
        let failed = if mode == "held-json" {
            durable_write(&target, b"cannot replace a directory").is_err()
        } else {
            ContainerWriter::save(
                &target,
                &Document::new(DocumentId::new()),
                &ContainerCaches::none(),
                &SaveMeta::default(),
            )
            .is_err()
        };
        assert!(failed, "replacement must fail for a nonempty directory");
        assert_eq!(
            fs::read(target.join("original")).unwrap(),
            b"existing destination data"
        );
        assert_eq!(fs::read(&active).unwrap(), held);
        assert_eq!(
            entries(root.path()),
            before,
            "only this failed write's temp may be removed"
        );
        stream.write_all(b"x").unwrap();
        child.finish();
    }
}

#[test]
fn unwinding_a_staged_write_removes_only_its_own_temp() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("settings.json");
    durable_write(&target, b"previous complete data").unwrap();
    let foreign = root.path().join(".settings.json.tmp-other-owner");
    fs::write(&foreign, HELD_BYTES).unwrap();
    let before = entries(root.path());
    let stopped = std::panic::catch_unwind(|| {
        let (_temporary, mut file) = OwnedTempFile::create(&target).unwrap();
        file.write_all(b"partial new data").unwrap();
        panic!("injected interruption before publication");
    });
    assert!(stopped.is_err());
    assert_eq!(fs::read(&target).unwrap(), b"previous complete data");
    assert_eq!(fs::read(&foreign).unwrap(), HELD_BYTES);
    assert_eq!(entries(root.path()), before);
}

fn write_snapshot(target: &Path, writer: u128, container: bool) {
    if container {
        let document = Document::new(DocumentId(uuid::Uuid::from_u128(writer)));
        ContainerWriter::save(
            target,
            &document,
            &ContainerCaches::none(),
            &SaveMeta::default(),
        )
        .unwrap();
    } else {
        durable_write(target, &json_payload(writer)).unwrap();
    }
}

fn json_payload(writer: u128) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "writer": writer as u64,
        "payload": char::from(b'a' + writer as u8).to_string().repeat(65_536 + writer as usize * 97),
    })).unwrap()
}

fn assert_whole_snapshot(target: &Path, container: bool, writers: &[u128]) {
    if container {
        let loaded = ContainerReader::open(target).expect("every observed archive is complete");
        assert!(writers
            .iter()
            .any(|writer| loaded.document().id == DocumentId(uuid::Uuid::from_u128(*writer))));
    } else {
        let bytes = fs::read(target).unwrap();
        assert!(
            writers.iter().any(|writer| bytes == json_payload(*writer)),
            "observed a torn JSON replacement"
        );
    }
}

fn run_snapshot_child(mode: &str, container: bool) -> bool {
    let Some((root, mut stream)) = child_connection(mode) else {
        return false;
    };
    let writer: u128 = std::env::var("ONECAD_IO_TEST_WRITER")
        .unwrap()
        .parse()
        .unwrap();
    let target = root.join(std::env::var("ONECAD_IO_TEST_TARGET").unwrap());
    stream.write_all(b"r").unwrap();
    stream.read_exact(&mut [0]).unwrap();
    for _ in 0..12 {
        write_snapshot(&target, writer, container);
    }
    true
}

fn observe_writers(children: &mut [WriterProcess], root: &Path, names: [&str; 2], container: bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while children
        .iter_mut()
        .any(|child| child.0.as_mut().unwrap().try_wait().unwrap().is_none())
    {
        assert!(
            Instant::now() < deadline,
            "concurrent writes did not finish"
        );
        for name in names {
            assert_whole_snapshot(&root.join(name), container, &[0, 1, 2]);
        }
        std::thread::yield_now();
    }
}

fn assert_finished_targets(root: &Path, names: [&str; 2], shared: bool, container: bool) {
    for (index, name) in names.into_iter().enumerate() {
        let expected = if shared {
            vec![1, 2]
        } else {
            vec![index as u128 + 1]
        };
        assert_whole_snapshot(&root.join(name), container, &expected);
    }
    let mut expected: Vec<_> = names.into_iter().map(std::ffi::OsString::from).collect();
    expected.sort();
    expected.dedup();
    assert_eq!(
        entries(root),
        expected,
        "successful writes leave no owned temps"
    );
}

fn concurrent_replacements(test: &str, mode: &str, container: bool) {
    for shared in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let names = if shared {
            ["shared", "shared"]
        } else {
            ["first", "second"]
        };
        for name in names {
            write_snapshot(&root.path().join(name), 0, container);
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut children = Vec::new();
        for (index, name) in names.into_iter().enumerate() {
            children.push(spawn_writer(
                test,
                mode,
                root.path(),
                listener.local_addr().unwrap(),
                &[
                    ("ONECAD_IO_TEST_WRITER", (index + 1).to_string()),
                    ("ONECAD_IO_TEST_TARGET", name.to_owned()),
                ],
            ));
        }
        let mut streams = [accept_writer(&listener), accept_writer(&listener)];
        for stream in &mut streams {
            stream.read_exact(&mut [0]).unwrap();
        }
        for stream in &mut streams {
            stream.write_all(b"x").unwrap();
        }
        observe_writers(&mut children, root.path(), names, container);
        for child in children {
            child.finish();
        }
        assert_finished_targets(root.path(), names, shared, container);
    }
}

#[test]
fn process_json_replacements_are_whole_for_shared_and_distinct_targets() {
    let mode = "json-replacements";
    if !run_snapshot_child(mode, false) {
        concurrent_replacements(
            "io::tests::process_json_replacements_are_whole_for_shared_and_distinct_targets",
            mode,
            false,
        );
    }
}

#[test]
fn process_container_replacements_are_whole_for_shared_and_distinct_targets() {
    let mode = "container-replacements";
    if !run_snapshot_child(mode, true) {
        concurrent_replacements(
            "io::tests::process_container_replacements_are_whole_for_shared_and_distinct_targets",
            mode,
            true,
        );
    }
}
