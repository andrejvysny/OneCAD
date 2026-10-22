use super::Result;
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

pub(super) struct Output {
    pub text: String,
    /// Standard output alone (machine-readable tool output such as JSON).
    pub stdout: String,
    pub status: Option<i32>,
}

/// Wall-clock and per-stream output bounds of one logged command.
#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub timeout: Duration,
    pub output_bytes: u64,
}

impl Limits {
    pub const DEFAULT: Self = Self {
        timeout: Duration::from_secs(600),
        output_bytes: 16 * 1024 * 1024,
    };
}

pub(super) fn run(
    directory: &Path,
    name: &str,
    program: &str,
    args: &[&str],
    environment: &BTreeMap<String, String>,
    success: bool,
) -> Result<Output> {
    run_limited(
        directory,
        name,
        program,
        args,
        environment,
        success,
        Limits::DEFAULT,
    )
}

/// [`run`] with explicit bounds (Xcode and kernel builds outlast the default).
pub(super) fn run_limited(
    directory: &Path,
    name: &str,
    program: &str,
    args: &[&str],
    environment: &BTreeMap<String, String>,
    success: bool,
    limits: Limits,
) -> Result<Output> {
    let stdout = directory.join(format!(".{name}.stdout"));
    let stderr = directory.join(format!(".{name}.stderr"));
    let mut child = Command::new(program)
        .args(args)
        .envs(environment)
        .stdout(Stdio::from(fs::File::create(&stdout)?))
        .stderr(Stdio::from(fs::File::create(&stderr)?))
        .spawn()?;
    let (status, exceeded) = wait(&mut child, &stdout, &stderr, limits)?;
    let standard = String::from_utf8_lossy(&fs::read(&stdout)?).into_owned();
    let output = format!("{standard}{}", String::from_utf8_lossy(&fs::read(&stderr)?));
    fs::remove_file(stdout)?;
    fs::remove_file(stderr)?;
    fs::write(directory.join(format!("{name}.log")), &output)?;
    record_command(directory, name, program, args, environment, status)?;
    print!("{output}");
    if exceeded {
        return Err(format!(
            "{name} exceeded {} second timeout or {} MiB output limit",
            limits.timeout.as_secs(),
            limits.output_bytes / (1024 * 1024)
        )
        .into());
    }
    if success && !status.success() {
        return Err(format!("{name} failed with status {:?}", status.code()).into());
    }
    Ok(Output {
        text: output,
        stdout: standard,
        status: status.code(),
    })
}

fn wait(
    child: &mut Child,
    stdout: &Path,
    stderr: &Path,
    limits: Limits,
) -> Result<(ExitStatus, bool)> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok((status, false));
        }
        if started.elapsed() >= limits.timeout
            || fs::metadata(stdout)?.len() > limits.output_bytes
            || fs::metadata(stderr)?.len() > limits.output_bytes
        {
            child.kill()?;
            return Ok((child.wait()?, true));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn record_command(
    directory: &Path,
    name: &str,
    program: &str,
    args: &[&str],
    environment: &BTreeMap<String, String>,
    status: ExitStatus,
) -> Result<()> {
    let selected: BTreeMap<_, _> = environment
        .iter()
        .filter(|(key, _)| {
            [
                "ONECAD_REQUIRE_WORKER",
                "ONECAD_REQUIRE_GPU",
                "ONECAD_WORKER_PATH",
                "WGPU_BACKEND",
                "CARGO_BUILD_JOBS",
                "CARGO_TARGET_DIR",
                "CARGO_PROFILE_TEST_DEBUG",
                "CARGO_INCREMENTAL",
                "DEVELOPER_DIR",
                "ONECAD_OCCT_ROOT",
                "ONECAD_SIM_UDID",
                "ONECAD_SIM_DEVICE_TYPE",
                "ONECAD_SIM_RUNTIME",
                "ONECAD_IOS_MAC_FIXTURE_OUT",
                "ONECAD_IOS_ACCEPTANCE_DIR",
                "ONECAD_IOS_ORACLE_REPORT",
                "TEST_RUNNER_ONECAD_RUN_ID",
                "TEST_RUNNER_ONECAD_MAC_FIXTURE",
            ]
            .contains(&key.as_str())
        })
        .collect();
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal: Option<i32> = None;
    fs::write(
        directory.join(format!("{name}.command.json")),
        serde_json::to_string_pretty(&json!({
            "program":program,"args":args,"environment":selected,"status":status.code(),"signal":signal
        }))?,
    )?;
    Ok(())
}
