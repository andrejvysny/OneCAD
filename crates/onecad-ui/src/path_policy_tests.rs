use super::*;

fn desktop(area: &Path) -> QaPaths {
    QaPaths::desktop_args(Vec::new(), area, &area.join("temporary")).unwrap()
}

#[test]
fn ordinary_desktop_startup_uses_persistent_profile_without_qa_environment() {
    let area = tempfile::tempdir().unwrap();
    let paths = desktop(area.path());
    assert_eq!(paths.policy, PathPolicy::Desktop);
    assert!(!paths.open_on_start);
    assert!(!paths.document.exists());
    assert_eq!(
        paths.documents,
        area.path().canonicalize().unwrap().join("Documents")
    );
    assert_eq!(
        paths.root,
        desktop_directories(&area.path().canonicalize().unwrap()).0
    );
    assert!(paths.data.is_dir() && paths.config.is_dir());
    assert!(paths.logs.is_absolute());
    assert!(!paths.root.join(".onecad-native-qa").exists());
    paths.require_launch_environment().unwrap();
}

#[test]
fn desktop_profile_lock_lives_until_last_owner_drops() {
    let area = tempfile::tempdir().unwrap();
    let paths = desktop(area.path());
    let lifetime = paths.clone();
    assert!(QaPaths::desktop_at(area.path(), &area.path().join("tmp"), None).is_err());
    drop(paths);
    assert!(QaPaths::desktop_at(area.path(), &area.path().join("tmp"), None).is_err());
    drop(lifetime);
    desktop(area.path());
}

#[test]
fn desktop_open_retains_external_source_and_only_explicit_qa_copies() {
    let area = tempfile::tempdir().unwrap();
    let source = area.path().join("external.ONECAD");
    fs::write(&source, b"source").unwrap();
    let paths = QaPaths::desktop_args(
        vec!["--open".into(), source.clone().into_os_string()],
        area.path(),
        &area.path().join("tmp"),
    )
    .unwrap();
    assert!(paths.open_on_start);
    assert_eq!(paths.document, source.canonicalize().unwrap());
    assert_eq!(paths.open_source(&source).unwrap(), paths.document);
    let qa = QaPaths::from_args(
        ["--qa-root".into(), area.path().join("qa").into_os_string()].into_iter(),
    )
    .unwrap();
    let copy = qa.open_source(&source).unwrap();
    assert!(copy.starts_with(&qa.documents));
    assert_ne!(copy, paths.document);
    assert_eq!(fs::read(source).unwrap(), b"source");
}

#[test]
fn desktop_accepts_external_destinations_while_qa_and_sandbox_remain_confined() {
    let area = tempfile::tempdir().unwrap();
    let paths = desktop(area.path());
    let outside = area.path().join("external.step");
    assert_eq!(
        paths.target(&outside).unwrap(),
        area.path().canonicalize().unwrap().join("external.step")
    );
    for policy in [PathPolicy::Qa, PathPolicy::Sandbox] {
        assert!(target_path(policy, &paths.documents, &outside).is_err());
        assert!(
            target_path(
                policy,
                &paths.documents,
                &paths.documents.join("part.onecad")
            )
            .is_ok()
        );
    }
    assert!(paths.target(Path::new("relative.onecad")).is_err());
    assert!(paths.target(&paths.documents).is_err());
}

#[cfg(unix)]
#[test]
fn desktop_rejects_leaf_symlinks_and_canonicalizes_selected_parent() {
    let area = tempfile::tempdir().unwrap();
    let paths = desktop(area.path());
    let file = area.path().join("original.onecad");
    fs::write(&file, b"preserve").unwrap();
    let link = area.path().join("linked.onecad");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    assert!(paths.target(&link).is_err());
    let alias = area.path().join("alias");
    std::os::unix::fs::symlink(&paths.documents, &alias).unwrap();
    assert_eq!(
        paths.target(&alias.join("chosen.onecad")).unwrap(),
        paths.documents.join("chosen.onecad")
    );
    assert_eq!(fs::read(file).unwrap(), b"preserve");
}

#[test]
fn desktop_parser_rejects_unknown_flags_and_missing_open_values() {
    let area = tempfile::tempdir().unwrap();
    for args in [vec!["--open".into()], vec!["--other".into(), "path".into()]] {
        assert!(QaPaths::desktop_args(args, area.path(), area.path()).is_err());
    }
    assert!(fs::read_dir(area.path()).unwrap().next().is_none());
}
