use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
};

pub(super) fn configuration_name(
    projection: super::Projection,
    theme: super::Theme,
    mode: super::DisplayMode,
) -> String {
    let projection = match projection {
        super::Projection::Perspective => "perspective",
        super::Projection::Orthographic => "orthographic",
    };
    let theme = match theme {
        super::Theme::Dark => "dark",
        super::Theme::Light => "light",
    };
    let mode = match mode {
        super::DisplayMode::Wireframe => "wireframe",
        super::DisplayMode::Shaded => "shaded",
        super::DisplayMode::ShadedEdges => "shaded-edges",
        super::DisplayMode::Xray => "xray",
    };
    format!("{projection}-{theme}-{mode}")
}

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn write(width: u32, height: u32, pixels: &[u8], case: &str, configuration: &str) {
    let directory = std::env::var_os("ONECAD_RENDER_CAPTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("onecad-render-quality-smoke"));
    write_to_directory(&directory, width, height, pixels, case, configuration);
}

fn write_to_directory(
    directory: &Path,
    width: u32,
    height: u32,
    pixels: &[u8],
    case: &str,
    configuration: &str,
) -> PathBuf {
    std::fs::create_dir_all(directory).unwrap();
    let (path, image, mut metadata) = reserve(directory, case, &SEQUENCE).unwrap();
    writeln!(
        metadata,
        "case: {case:?}\nsize: {width}x{height}\n{configuration}\n{}",
        source_provenance()
    )
    .unwrap();
    metadata.sync_all().unwrap();
    let mut encoder = png::Encoder::new(image, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(pixels).unwrap();
    writer.finish().unwrap();
    eprintln!("renderer capture: {} case={case:?}", path.display());
    path
}

fn portable_case(case: &str) -> String {
    case.chars()
        .take(72)
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn reserve(
    directory: &Path,
    case: &str,
    sequence: &AtomicU64,
) -> io::Result<(PathBuf, File, File)> {
    loop {
        let index = sequence.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!("capture-{index:06}-{}.png", portable_case(case)));
        let image = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.with_extension("txt"))
        {
            Ok(metadata) => return Ok((path, image, metadata)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                drop(image);
                std::fs::remove_file(&path)?;
            }
            Err(error) => return Err(error),
        }
    }
}

fn source_provenance() -> &'static str {
    static SOURCE: OnceLock<String> = OnceLock::new();
    SOURCE.get_or_init(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let git = |arguments: &[&str]| {
            let output = std::process::Command::new("git")
                .args(arguments)
                .current_dir(&root)
                .output()
                .expect("capture source provenance requires git");
            assert!(output.status.success(), "capture source provenance failed");
            String::from_utf8(output.stdout).unwrap()
        };
        format!(
            "source_commit: {}source_status:\n{}",
            git(&["rev-parse", "HEAD"]),
            git(&["status", "--porcelain=v1", "--untracked-files=normal"])
        )
    })
}

#[test]
fn feature_projection_theme_and_mode_matrix_has_distinct_portable_labels() {
    use super::{DisplayMode, Projection, Theme};
    let mut names = std::collections::HashSet::new();
    for depth in ["fallback", "stencil"] {
        for projection in [Projection::Perspective, Projection::Orthographic] {
            for theme in [Theme::Dark, Theme::Light] {
                for mode in [
                    DisplayMode::Wireframe,
                    DisplayMode::Shaded,
                    DisplayMode::ShadedEdges,
                    DisplayMode::Xray,
                ] {
                    let label = format!("{depth}-{}", configuration_name(projection, theme, mode));
                    assert_eq!(portable_case(&label), label);
                    assert!(names.insert(label));
                }
            }
        }
    }
    assert_eq!(names.len(), 32);
}

#[test]
fn png_and_metadata_preserve_configuration_for_each_capture() {
    let directory = test_directory("written");
    let pixels = [255, 0, 0, 255, 0, 255, 0, 255];
    let case = "Features(DEPTH32FLOAT_STENCIL8 | FLOAT32_BLENDABLE):?";
    for configuration in [
        "features: fallback\nprojection: Perspective\ntheme: Dark\nadapter: Apple M4 Pro",
        "features: stencil\nprojection: Orthographic\ntheme: Light\nadapter: Apple M4 Pro",
    ] {
        let path = write_to_directory(&directory, 2, 1, &pixels, case, configuration);
        let metadata = std::fs::read_to_string(path.with_extension("txt")).unwrap();
        assert!(metadata.contains(&format!("case: {case:?}")));
        assert!(metadata.contains(configuration));
        assert!(metadata.contains("source_commit: "));
        assert!(metadata.contains("source_status:\n"));
        let mut reader = png::Decoder::new(io::BufReader::new(File::open(path).unwrap()))
            .read_info()
            .unwrap();
        let mut decoded = [0; 8];
        let info = reader.next_frame(&mut decoded).unwrap();
        assert_eq!((info.width, info.height), (2, 1));
        assert_eq!(pixels, decoded);
    }
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 4);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn capture_names_are_portable_bounded_and_keep_original_metadata() {
    let directory = test_directory("portable");
    let sequence = AtomicU64::new(0);
    for case in [
        "Features(DEPTH: A|B)?/\\*\"<>\0",
        "CON",
        "",
        "é",
        &"x".repeat(400),
    ] {
        let (path, _, mut metadata) = reserve(&directory, case, &sequence).unwrap();
        writeln!(metadata, "case: {case:?}").unwrap();
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert!(filename.len() <= 100);
        assert!(filename
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')));
        assert_eq!(
            std::fs::read_to_string(path.with_extension("txt")).unwrap(),
            format!("case: {case:?}\n")
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn duplicate_cases_and_restarted_sequences_never_overwrite_captures() {
    let directory = test_directory("collision");
    let sequence = AtomicU64::new(0);
    let (first, mut image, _) = reserve(&directory, "case:a", &sequence).unwrap();
    image.write_all(b"immutable first capture").unwrap();
    let (second, _, _) = reserve(&directory, "case/a", &sequence).unwrap();
    let (restarted, _, _) = reserve(&directory, "case:a", &AtomicU64::new(0)).unwrap();
    assert_ne!(first, second);
    assert_ne!(first, restarted);
    assert_ne!(second, restarted);
    assert_eq!(std::fs::read(first).unwrap(), b"immutable first capture");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn orphan_metadata_is_preserved_and_file_errors_fail() {
    let directory = test_directory("metadata");
    let metadata = directory.join("capture-000000-case.txt");
    std::fs::write(&metadata, "existing metadata").unwrap();
    let (path, _, _) = reserve(&directory, "case", &AtomicU64::new(0)).unwrap();
    assert_eq!(path.file_name().unwrap(), "capture-000001-case.png");
    assert_eq!(
        std::fs::read_to_string(metadata).unwrap(),
        "existing metadata"
    );
    let error = reserve(&path, "case", &AtomicU64::new(0)).unwrap_err();
    assert_ne!(error.kind(), io::ErrorKind::AlreadyExists);
    std::fs::remove_dir_all(directory).unwrap();
}

fn test_directory(case: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "onecad-render-artifacts-{}-{}-{case}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&directory).unwrap();
    directory
}
