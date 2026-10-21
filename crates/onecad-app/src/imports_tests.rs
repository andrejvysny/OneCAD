//! Reference-counting behavior of the shared import-blob files.

use super::*;

#[test]
fn shared_blob_survives_first_drop_and_dies_with_last() {
    let sha = sha256_hex(b"refcount-a");
    let mut a = ImportWorkspace::new(DocumentId::new());
    let mut b = ImportWorkspace::new(DocumentId::new());
    let pa = a
        .materialize(&sha, ImportSourceCodec::Brep, b"refcount-a")
        .expect("materialize a");
    let pb = b
        .materialize(&sha, ImportSourceCodec::Brep, b"refcount-a")
        .expect("materialize b");
    assert_eq!(pa, pb);

    drop(a);
    assert!(pb.is_file(), "the second holder still needs the file");
    assert_eq!(resolve_blob_path(&sha), pb.to_string_lossy());

    drop(b);
    assert!(!pb.exists(), "the last holder's drop deletes the file");
    assert_eq!(resolve_blob_path(&sha), "");
}

#[test]
fn one_workspace_holds_a_digest_once() {
    let sha = sha256_hex(b"refcount-b");
    let mut ws = ImportWorkspace::new(DocumentId::new());
    let first = ws
        .materialize(&sha, ImportSourceCodec::Step, b"refcount-b")
        .expect("materialize");
    let again = ws
        .materialize(&sha, ImportSourceCodec::Step, b"refcount-b")
        .expect("re-materialize");
    assert_eq!(first, again);
    drop(ws);
    assert!(!first.exists(), "one drop must release the single hold");
    assert_eq!(resolve_blob_path(&sha), "");
}

#[test]
fn sweep_leaves_the_live_process_shared_dir_alone() {
    let sha = sha256_hex(b"refcount-c");
    let mut ws = ImportWorkspace::new(DocumentId::new());
    let path = ws
        .materialize(&sha, ImportSourceCodec::Brep, b"refcount-c")
        .expect("materialize");
    // Zero max age makes every directory stale by mtime, so only the live-pid
    // check can be what spares it.
    std::thread::sleep(Duration::from_millis(10));
    sweep_stale_workspaces(Duration::ZERO);
    assert!(path.is_file(), "a live process's shared dir must survive");
}
