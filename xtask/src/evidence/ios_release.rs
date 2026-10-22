//! Release product checks of `ios-simulator-gate` step 9: no test-hook symbol,
//! no worker test verb, the expected `LC_BUILD_VERSION` and only system
//! libraries.
use super::Result;

/// Mach-O platform of one Release product.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Platform {
    Device,
    Simulator,
}

impl Platform {
    pub fn sdk(self) -> &'static str {
        match self {
            Self::Device => "iphoneos",
            Self::Simulator => "iphonesimulator",
        }
    }

    pub fn destination(self) -> &'static str {
        match self {
            Self::Device => "generic/platform=iOS",
            Self::Simulator => "generic/platform=iOS Simulator",
        }
    }

    /// `otool -l` prints the number (older) or the name (newer cctools).
    fn accepts(self, platform: &str) -> bool {
        match self {
            Self::Device => matches!(platform, "2" | "IOS" | "iOS"),
            Self::Simulator => matches!(platform, "7" | "IOSSIMULATOR" | "iOSSimulator"),
        }
    }
}

/// Worker test verbs that only a `--test-hooks` kernel bundle compiles in.
pub(super) const TEST_VERBS: [&str; 2] = ["Debug.Busy", "__slow"];

/// `nm -j` output: no `onecad_ios_test_` symbol of any kind.
pub(super) fn verify_symbols(nm: &str) -> Result<()> {
    match nm.lines().find(|line| line.contains("onecad_ios_test_")) {
        Some(symbol) => Err(format!("Release product carries test hook {}", symbol.trim()).into()),
        None if nm.trim().is_empty() => Err("nm listed no symbols".into()),
        None => Ok(()),
    }
}

/// The product's bytes contain no worker test verb (what `strings` would show).
pub(super) fn verify_strings(binary: &[u8]) -> Result<()> {
    for verb in TEST_VERBS {
        if binary
            .windows(verb.len())
            .any(|window| window == verb.as_bytes())
        {
            return Err(format!("Release product contains worker test verb {verb}").into());
        }
    }
    Ok(())
}

/// `otool -l`: exactly one `LC_BUILD_VERSION`, for `platform`, `minos 18.0`.
pub(super) fn verify_build_version(otool: &str, platform: Platform) -> Result<()> {
    let mut found = Vec::new();
    let mut lines = otool.lines().map(str::trim);
    while let Some(line) = lines.next() {
        if line != "cmd LC_BUILD_VERSION" {
            continue;
        }
        let (mut os, mut minos) = (None, None);
        for field in lines.by_ref().take(6) {
            if let Some(value) = field.strip_prefix("platform ") {
                os = Some(value.trim().to_string());
            } else if let Some(value) = field.strip_prefix("minos ") {
                minos = Some(value.trim().to_string());
            }
        }
        found.push((os, minos));
    }
    match found.as_slice() {
        [(Some(os), Some(minos))] if platform.accepts(os) && minos == "18.0" => Ok(()),
        _ => Err(format!(
            "{} product LC_BUILD_VERSION is {found:?}, expected one {platform:?} minos 18.0",
            platform.sdk()
        )
        .into()),
    }
}

/// `otool -L`: every dependency is a system library or framework.
pub(super) fn verify_libraries(otool: &str) -> Result<()> {
    let mut dependencies = otool
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .peekable();
    if dependencies.peek().is_none() {
        return Err("otool -L listed no dependencies".into());
    }
    for line in dependencies {
        let path = line.split(" (").next().unwrap_or(line);
        if !(path.starts_with("/usr/lib/") || path.starts_with("/System/Library/")) {
            return Err(format!("Release product links a non-system library: {path}").into());
        }
    }
    Ok(())
}
