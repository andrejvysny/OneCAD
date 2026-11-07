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
fn dropping_a_claim_releases_only_that_lease() {
    let table = Arc::new(Reservations::default());
    let area = tempfile::tempdir().unwrap();
    let path = &area.path().join("A.onecad");
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

#[test]
fn relative_basename_claim_survives_target_creation() {
    const CHILD: &str = "ONECAD_RELATIVE_CLAIM_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let table = Arc::new(Reservations::default());
        let target = Path::new("target.onecad");
        let claim = table.claim_path(1, target).unwrap();
        std::fs::write(target, b"saved").unwrap();
        let absolute = std::env::current_dir().unwrap().join(target);
        assert!(table.claim_path(2, &absolute).is_err());
        assert!(table.claim_path(2, target).is_err());
        drop(claim);
        assert!(table.claim_path(2, &absolute).is_ok());
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .current_dir(tmp.path())
        .env(CHILD, "1")
        .args([
            "--exact",
            "reservations::tests::relative_basename_claim_survives_target_creation",
        ])
        .status()
        .unwrap();
    assert!(status.success());
}

#[cfg(unix)]
#[test]
fn symlink_entry_and_referent_remain_claimed_across_replacement() {
    use std::os::unix::fs::symlink;
    let table = Arc::new(Reservations::default());
    let dir = tempfile::tempdir().unwrap();
    let referent = dir.path().join("referent.onecad");
    let link = dir.path().join("link.onecad");
    std::fs::write(&referent, b"old").unwrap();
    symlink(&referent, &link).unwrap();
    let first = table.claim_path(1, &link).unwrap();
    let second = table.claim_path(1, &link).unwrap();
    assert!(table.claim_path(2, &referent).is_err());
    let replacement = dir.path().join("replacement");
    std::fs::write(&replacement, b"new").unwrap();
    std::fs::rename(&replacement, &link).unwrap();
    assert!(table.claim_path(2, &link).is_err());
    assert!(table.claim_path(2, &referent).is_err());
    drop(first);
    assert!(table.claim_path(2, &link).is_err());
    assert!(table.claim_path(2, &referent).is_err());
    drop(second);
    assert!(table.claim_path(2, &link).is_ok());
    assert!(table.claim_path(2, &referent).is_ok());
}

#[test]
fn nonexistent_parent_is_refused_without_creating_it() {
    let table = Arc::new(Reservations::default());
    let area = tempfile::tempdir().unwrap();
    let parent = area.path().join("missing");
    let target = parent.join("model.onecad");
    assert!(matches!(table.claim_path(1, &target), Err(ApiError::Io(_))));
    assert!(!parent.exists());
}

#[cfg(unix)]
#[test]
fn conflicting_referent_does_not_partially_claim_entry() {
    use std::os::unix::fs::symlink;
    let table = Arc::new(Reservations::default());
    let dir = tempfile::tempdir().unwrap();
    let referent = dir.path().join("referent.onecad");
    let link = dir.path().join("link.onecad");
    std::fs::write(&referent, b"old").unwrap();
    symlink(&referent, &link).unwrap();
    let claim = table.claim_path(1, &referent).unwrap();
    assert!(table.claim_path(2, &link).is_err());
    let entry = Key::Path(dir.path().canonicalize().unwrap().join("link.onecad"));
    assert!(!table.owners().contains_key(&entry));
    drop(claim);
    assert!(table.claim_path(2, &link).is_ok());
}
