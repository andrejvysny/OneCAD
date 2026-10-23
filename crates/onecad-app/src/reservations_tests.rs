use super::*;

fn id(seed: u128) -> DocumentId {
    DocumentId(uuid::Uuid::from_u128(seed))
}

#[test]
fn a_path_open_in_one_host_is_refused_to_another_and_names_the_owner() {
    let table = Arc::new(Reservations::default());
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Part.onecad");
    std::fs::write(&file, b"x").unwrap();
    let _a = table.claim_path(1, &file).unwrap();
    match table.claim_path(2, &file) {
        Err(ApiError::AlreadyOpen { host, .. }) => assert_eq!(host, 1),
        other => panic!("expected AlreadyOpen, got {other:?}"),
    }
    let _again = table
        .claim_path(1, &file)
        .expect("the owner may claim again");
}

#[test]
fn aliases_of_one_file_collide() {
    let table = Arc::new(Reservations::default());
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Part.onecad");
    std::fs::write(&file, b"x").unwrap();
    let alias = dir.path().join(".").join("Part.onecad");
    let _a = table.claim_path(1, &file).unwrap();
    assert!(table.claim_path(2, &alias).is_err());
}

#[test]
fn a_save_target_that_does_not_exist_yet_is_still_comparable() {
    let table = Arc::new(Reservations::default());
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("New.onecad");
    let _a = table.claim_path(1, &target).unwrap();
    assert!(table
        .claim_path(2, &dir.path().join(".").join("New.onecad"))
        .is_err());
}

#[test]
fn missing_parent_traversal_cannot_bypass_an_existing_path_claim() {
    let table = Arc::new(Reservations::default());
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("held.onecad");
    std::fs::write(&target, b"owned").unwrap();
    let _claim = table.claim_path(1, &target).unwrap();
    let alias = dir.path().join("missing/../held.onecad");
    assert!(matches!(
        table.claim_path(2, &alias),
        Err(ApiError::AlreadyOpen { host: 1, .. })
    ));
    assert_eq!(table.path_owner(&alias), Some(1));
    assert!(!dir.path().join("missing").exists());
}

#[cfg(unix)]
#[test]
fn missing_parents_preserve_symlink_parent_traversal_when_claiming_future_targets() {
    let table = Arc::new(Reservations::default());
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("destination");
    std::fs::create_dir_all(destination.join("child")).unwrap();
    std::os::unix::fs::symlink(destination.join("child"), dir.path().join("alias")).unwrap();
    let target = destination.join("new/deep/part.onecad");
    let _claim = table.claim_path(1, &target).unwrap();
    let alias = dir.path().join("alias/../new/deep/part.onecad");
    assert_eq!(table.path_owner(&alias), Some(1));
    assert!(matches!(
        table.claim_path(2, &alias),
        Err(ApiError::AlreadyOpen { host: 1, .. })
    ));
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    assert_eq!(table.path_owner(&target), Some(1));
}

#[test]
fn dropping_a_claim_releases_only_that_lease() {
    let table = Arc::new(Reservations::default());
    let path = Path::new("/nonexistent/onecad/A.onecad");
    let first = table.claim_path(1, path).unwrap();
    let second = table.claim_path(1, path).unwrap();
    drop(first);
    assert_eq!(table.path_owner(path), Some(1), "one lease still holds it");
    drop(second);
    assert_eq!(table.path_owner(path), None);
    let _b = table
        .claim_path(2, path)
        .expect("free after the last lease");
}

#[test]
fn a_duplicate_document_identity_is_refused() {
    let table = Arc::new(Reservations::default());
    let _a = table.claim_document(1, id(7)).unwrap();
    assert!(matches!(
        table.claim_document(2, id(7)),
        Err(ApiError::DuplicateDocument { host: 1, .. })
    ));
}

#[test]
fn the_cap_counts_hosts_not_claims() {
    let table = Arc::new(Reservations::default());
    let held: Vec<_> = (0..MAX_OPEN_DOCUMENTS as u64)
        .map(|host| table.claim_document(host, id(u128::from(host))).unwrap())
        .collect();
    assert_eq!(table.open_documents(), MAX_OPEN_DOCUMENTS);
    assert!(matches!(
        table.claim_document(99, id(99)),
        Err(ApiError::TooManyDocuments(_))
    ));
    let _replacement = table
        .claim_document(0, id(1000))
        .expect("an occupying host may stage its replacement document");
    drop(held);
    let _ok = table.claim_document(99, id(99)).expect("room after close");
}

#[test]
fn a_poisoned_table_still_serves() {
    let table = Arc::new(Reservations::default());
    let poisoner = table.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poisoner.owners.lock().unwrap();
        panic!("poison");
    })
    .join();
    let _claim = table.claim_document(1, id(1)).unwrap();
    assert_eq!(table.open_documents(), 1);
}
