//! Thumbnail decoding and the library service lane against a seeded library.
use super::{Reply, Request, draft::ParamDraft, service, thumbs};
use onecad_app::{
    host::{AppEvent, DialogPort, EventSink, HostContext, HostPaths},
    state::AppState,
    worker::PendingBackend,
};
use onecad_core::document::record::ComponentParamValue as Value;
use std::{collections::BTreeMap, sync::Arc};

// ---- thumbnails ----

fn write_png(path: &std::path::Path, w: u32, h: u32) {
    let file = std::fs::File::create(path).expect("create png");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("header");
    let data: Vec<u8> = (0..w * h).flat_map(|i| [(i % 251) as u8, 7, 200]).collect();
    writer.write_image_data(&data).expect("pixels");
}

#[test]
fn preview_png_decodes_to_rgba_and_large_ones_shrink() {
    let dir = tempfile::tempdir().expect("tempdir");
    let small = dir.path().join("small.png");
    write_png(&small, 40, 30);
    let decoded = thumbs::decode(&small).expect("decode");
    assert_eq!(decoded.size, [40, 30]);
    assert_eq!(decoded.rgba.len(), 40 * 30 * 4);
    assert_eq!(decoded.rgba[3], 255, "RGB gains an opaque alpha");

    let large = dir.path().join("large.png");
    write_png(&large, 1500, 900);
    let shrunk = thumbs::decode(&large).expect("decode");
    assert!(
        shrunk.size[0].max(shrunk.size[1]) <= 512,
        "{:?}",
        shrunk.size
    );
    assert_eq!(shrunk.rgba.len(), shrunk.size[0] * shrunk.size[1] * 4);

    let broken = dir.path().join("broken.png");
    std::fs::write(&broken, b"not a png").expect("write");
    assert!(thumbs::decode(&broken).is_err());
    assert!(thumbs::decode(&dir.path().join("missing.png")).is_err());
}

#[test]
fn decoded_previews_become_textures_and_the_cache_is_bounded() {
    let ctx = egui::Context::default();
    let mut cache = thumbs::Cache::default();
    let image = || thumbs::Decoded {
        size: [2, 2],
        rgba: vec![255; 16],
    };
    for i in 0..140 {
        cache.settle(&ctx, (format!("c{i}"), "1".into()), Ok(image()));
    }
    assert!(cache.len() <= 128);
    // The oldest were evicted back to absent, so they can be decoded again.
    assert!(cache.texture(&("c0".into(), "1".into())).is_none());
    assert!(cache.absent(&("c0".into(), "1".into())));
    assert!(cache.texture(&("c139".into(), "1".into())).is_some());
    cache.clear();
    assert!(cache.absent(&("c139".into(), "1".into())));
}

// ---- real service ----

struct Ports;
impl EventSink for Ports {
    fn emit(&self, _: AppEvent) -> Result<(), String> {
        Ok(())
    }
}
#[async_trait::async_trait]
impl DialogPort for Ports {
    async fn pick(&self, _: onecad_app::host::FileDialog) -> Vec<String> {
        Vec::new()
    }
}

fn state_without_worker() -> AppState {
    AppState::new(Arc::new(|| {
        let backend = Arc::new(PendingBackend);
        (
            backend.clone(),
            backend.clone(),
            backend.clone(),
            backend.clone(),
            backend.clone(),
            backend.clone(),
            backend.clone(),
            backend.clone(),
            backend.clone(),
            backend,
        )
    }))
}

/// A host whose library is `<data>/library`, seeded with the built-in packages.
/// `None` when `ONECAD_LIBRARY_ROOT` redirects the service elsewhere; the test then
/// must not write into a directory it did not create.
fn seeded_host(data: &std::path::Path) -> Option<HostContext> {
    let ports = Arc::new(Ports);
    let host = HostContext {
        paths: HostPaths {
            app_data: Some(data.to_path_buf()),
            app_config: Some(data.join("config")),
        },
        events: ports.clone(),
        dialogs: ports,
    };
    let root = onecad_app::library::library_root(&host)?;
    if root != data.join("library") {
        eprintln!("ONECAD_LIBRARY_ROOT is set; skipping the seeded service test");
        return None;
    }
    onecad_app::library::seed_and_reindex_at(&root);
    Some(host)
}

fn search(text: Option<&str>) -> Request {
    Request::Search {
        sequence: 1,
        query: onecad_app::dto::SearchQueryDto {
            text: text.map(str::to_owned),
            latest_only: true,
            ..Default::default()
        },
    }
}

#[tokio::test]
async fn real_service_serves_the_seeded_iso4762_entry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(host) = seeded_host(dir.path()) else {
        return;
    };
    let state = state_without_worker();

    let Reply::Searched(all) = service::run(&host, &state, search(None))
        .await
        .expect("search")
    else {
        panic!("search reply");
    };
    let screw = all
        .iter()
        .find(|c| c.id == "onecad.std.iso4762")
        .expect("seeded entry");
    assert_eq!(screw.name, "Socket Head Cap Screw");
    assert_eq!(screw.generator_id.as_deref(), Some("iso4762"));

    let Reply::Searched(none) = service::run(&host, &state, search(Some("no such part")))
        .await
        .expect("search")
    else {
        panic!("search reply");
    };
    assert!(none.is_empty());

    // The draft is built from the real DTO, including the TOML-typed defaults.
    let draft = ParamDraft::from_component(screw);
    assert_eq!(draft.designation().as_deref(), Some("ISO 4762 M6x20"));
    assert!(draft.resolve().is_ok());

    let detail = Request::Detail {
        sequence: 2,
        id: screw.id.clone(),
        version: screw.version.clone(),
    };
    let Reply::Detail(detail) = service::run(&host, &state, detail).await.expect("detail") else {
        panic!("detail reply");
    };
    assert_eq!(detail.versions, std::slice::from_ref(&screw.version));
    assert_eq!(detail.component.id, screw.id);

    let missing = Request::Detail {
        sequence: 3,
        id: "nope".into(),
        version: "1.0.0".into(),
    };
    let error = service::run(&host, &state, missing)
        .await
        .expect_err("unknown component");
    assert!(error.contains("unknown component"), "{error}");

    let Reply::Reindexed(report) = service::run(&host, &state, Request::Reindex { sequence: 4 })
        .await
        .expect("reindex")
    else {
        panic!("reindex reply");
    };
    assert_eq!(report.indexed, report.total - report.skipped.len());
    assert!(report.indexed >= 1);
}

#[tokio::test]
async fn real_service_refuses_a_preview_without_a_worker_honestly() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(host) = seeded_host(dir.path()) else {
        return;
    };
    let request = Request::Preview {
        sequence: 5,
        id: "onecad.std.iso4762".into(),
        version: "1.0.0".into(),
        params: BTreeMap::from([("length".into(), Value::Number(20.0))]),
    };
    let error = service::run(&host, &state_without_worker(), request)
        .await
        .expect_err("no worker");
    assert!(error.contains("no geometry worker"), "{error}");
}

#[tokio::test]
async fn real_service_decodes_package_previews_off_the_actor() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(host) = seeded_host(dir.path()) else {
        return;
    };
    let package = std::fs::read_dir(dir.path().join("library"))
        .expect("library")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.join("component.toml").is_file())
        .expect("a seeded package directory");
    write_png(&package.join("preview.png"), 64, 48);

    let Reply::Searched(all) = service::run(&host, &state_without_worker(), search(None))
        .await
        .expect("search")
    else {
        panic!("search reply");
    };
    let with_preview = all
        .iter()
        .find(|c| c.preview_path.is_some())
        .expect("preview found on disk");
    let items = vec![(
        (with_preview.id.clone(), with_preview.version.clone()),
        with_preview.preview_path.clone().expect("path").into(),
    )];
    let request = Request::Thumbnails { sequence: 6, items };
    let Reply::Thumbnails(decoded) = service::run(&host, &state_without_worker(), request)
        .await
        .expect("thumbs")
    else {
        panic!("thumbnail reply");
    };
    assert_eq!(decoded[0].1.as_ref().expect("decoded").size, [64, 48]);
}

#[tokio::test]
async fn a_panicking_request_becomes_an_error_reply() {
    let result: Result<u8, String> = service::guarded(async {
        if true {
            panic!("worker exploded");
        }
        Ok(1)
    })
    .await;
    assert_eq!(
        result,
        Err("library service stopped unexpectedly: worker exploded".to_owned())
    );
    let fine: Result<u8, String> = service::guarded(async { Ok(7) }).await;
    assert_eq!(fine, Ok(7));
    let failed: Result<u8, String> = service::guarded(async { Err("plain".to_owned()) }).await;
    assert_eq!(failed, Err("plain".to_owned()));
}

#[test]
fn unusable_preview_geometry_is_an_error_not_a_size() {
    use service::{BodyBounds, combine};
    let body = |min: [f32; 3], max: [f32; 3], triangles| BodyBounds {
        min,
        max,
        triangles,
    };
    let none = "preview returned no usable geometry".to_owned();
    assert_eq!(combine(&[]).unwrap_err(), none);
    assert_eq!(combine(&[body([0.0; 3], [1.0; 3], 0)]).unwrap_err(), none);
    assert_eq!(
        combine(&[body([0.0; 3], [f32::NAN, 1.0, 1.0], 12)]).unwrap_err(),
        none
    );
    assert_eq!(
        combine(&[
            body([0.0; 3], [1.0; 3], 12),
            body([f32::NEG_INFINITY; 3], [1.0; 3], 12)
        ])
        .unwrap_err(),
        none
    );
    let ok = combine(&[
        body([0.0; 3], [2.0, 1.0, 1.0], 12),
        body([-1.0, 0.0, 0.0], [1.0; 3], 6),
    ])
    .unwrap();
    assert_eq!((ok.size, ok.bodies, ok.triangles), ([3.0, 1.0, 1.0], 2, 18));
}

#[test]
fn a_preview_declaring_a_huge_image_is_refused_before_allocation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huge.png");
    let file = std::fs::File::create(&path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), 9000, 9000);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::One);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&vec![0u8; 1125 * 9000]).unwrap();
    drop(writer);
    let error = thumbs::decode(&path).unwrap_err();
    assert!(error.contains("limit is"), "{error}");
}

#[test]
fn settling_a_key_twice_and_eviction_keep_the_cache_consistent() {
    let ctx = egui::Context::default();
    let mut cache = thumbs::Cache::default();
    let image = |n| thumbs::Decoded {
        size: [n, n],
        rgba: vec![255; n * n * 4],
    };
    let key = ("a".to_owned(), "1".to_owned());
    cache.settle(&ctx, key.clone(), Ok(image(4)));
    cache.settle(&ctx, key.clone(), Ok(image(2)));
    assert_eq!(cache.len(), 1, "no duplicate order entry");
    assert_eq!(
        cache.bytes(),
        2 * 2 * 4,
        "replaced handle's bytes were subtracted"
    );
    for i in 0..130 {
        cache.settle(&ctx, (format!("k{i}"), "1".into()), Ok(image(1)));
    }
    // Evicted entries are absent (re-decodable), unlike a decode failure.
    assert!(cache.absent(&key));
    cache.settle(&ctx, ("bad".into(), "1".into()), Err("corrupt".into()));
    assert!(!cache.absent(&("bad".into(), "1".into())));
}
