//! Simulator selection and run identity for `ios-simulator-gate` (steps 2 and
//! 4). Selection reuses or creates `OneCAD Gate <device> <runtime>` and never
//! erases, shuts down or deletes another simulator.
use super::Result;
use serde_json::Value;

pub(super) const DEFAULT_DEVICE_TYPE: &str = "iPad Pro 13-inch (M5)";

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Runtime {
    pub identifier: String,
    pub version: String,
}

fn version_key(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

/// `simctl list runtimes -j`: the requested runtime (identifier, name or
/// version), else the newest available iOS (iPadOS) runtime.
pub(super) fn runtime(runtimes: &Value, requested: Option<&str>) -> Result<Runtime> {
    let mut available = runtimes["runtimes"]
        .as_array()
        .ok_or("simctl runtimes JSON without runtimes")?
        .iter()
        .filter(|r| r["isAvailable"] == true && r["platform"] == "iOS");
    let chosen = match requested {
        Some(wanted) => available.rfind(|r| {
            [&r["identifier"], &r["name"], &r["version"]]
                .iter()
                .any(|v| **v == wanted)
        }),
        None => available.max_by_key(|r| version_key(r["version"].as_str().unwrap_or_default())),
    };
    let chosen = chosen.ok_or("no matching available iPadOS simulator runtime")?;
    Ok(Runtime {
        identifier: chosen["identifier"]
            .as_str()
            .ok_or("runtime identifier")?
            .into(),
        version: chosen["version"].as_str().ok_or("runtime version")?.into(),
    })
}

/// `simctl list devicetypes -j`: identifier of the device type named `name`
/// (or given by identifier).
pub(super) fn device_type(types: &Value, name: &str) -> Result<String> {
    types["devicetypes"]
        .as_array()
        .ok_or("simctl devicetypes JSON without devicetypes")?
        .iter()
        .find(|t| t["name"] == name || t["identifier"] == name)
        .and_then(|t| t["identifier"].as_str().map(str::to_string))
        .ok_or_else(|| format!("simulator device type {name} is not installed").into())
}

/// `OneCAD Gate iPad Pro 13 M5 26.5` for `iPad Pro 13-inch (M5)` on 26.5.
pub(super) fn gate_name(device: &str, runtime: &Runtime) -> String {
    let short = device.replace("-inch", "").replace(['(', ')'], "");
    format!("OneCAD Gate {short} {}", runtime.version)
}

/// `simctl list devices available -j`: UDID of the available device `name`
/// under `runtime`.
pub(super) fn find_named(devices: &Value, runtime: &Runtime, name: &str) -> Option<String> {
    devices["devices"][&runtime.identifier]
        .as_array()?
        .iter()
        .find(|d| d["name"] == name && d["isAvailable"] != false)
        .and_then(|d| d["udid"].as_str().map(str::to_string))
}

/// The runtime identifier and name of the available device `udid`.
pub(super) fn find_udid(devices: &Value, udid: &str) -> Option<(String, String)> {
    let runtimes = devices["devices"].as_object()?;
    runtimes.iter().find_map(|(runtime, list)| {
        list.as_array()?
            .iter()
            .find(|d| d["udid"] == udid)
            .map(|d| {
                (
                    runtime.clone(),
                    d["name"].as_str().unwrap_or_default().to_string(),
                )
            })
    })
}

/// Proleptic Gregorian date of a day count since 1970-01-01 (H. Hinnant).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// `<yyyymmddThhmmssZ>-<sha8>-<16 hex random>`.
pub(super) fn run_id(unix_seconds: u64, source: &str, random: [u8; 8]) -> String {
    let seconds = unix_seconds as i64;
    let (year, month, day) = civil(seconds.div_euclid(86_400));
    let time = seconds.rem_euclid(86_400);
    let random: String = random.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z-{}-{random}",
        time / 3600,
        time % 3600 / 60,
        time % 60,
        &source[..8.min(source.len())]
    )
}

pub(super) fn random_bytes() -> Result<[u8; 8]> {
    use std::io::Read;
    let mut bytes = [0; 8];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes)
}
