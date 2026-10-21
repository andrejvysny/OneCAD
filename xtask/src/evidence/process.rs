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
    pub status: Option<i32>,
}

pub(super) fn run(
    directory: &Path,
    name: &str,
    program: &str,
    args: &[&str],
    environment: &BTreeMap<String, String>,
    success: bool,
) -> Result<Output> {
    let stdout = directory.join(format!(".{name}.stdout"));
    let stderr = directory.join(format!(".{name}.stderr"));
    let mut child = Command::new(program)
        .args(args)
        .envs(environment)
        .stdout(Stdio::from(fs::File::create(&stdout)?))
        .stderr(Stdio::from(fs::File::create(&stderr)?))
        .spawn()?;
    let (status, exceeded) = wait(&mut child, &stdout, &stderr)?;
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&fs::read(&stdout)?),
        String::from_utf8_lossy(&fs::read(&stderr)?)
    );
    fs::remove_file(stdout)?;
    fs::remove_file(stderr)?;
    fs::write(directory.join(format!("{name}.log")), &output)?;
    record_command(directory, name, program, args, environment, status)?;
    print!("{output}");
    if exceeded {
        return Err(format!("{name} exceeded 600 second timeout or 16 MiB output limit").into());
    }
    if success && !status.success() {
        return Err(format!("{name} failed with status {:?}", status.code()).into());
    }
    Ok(Output {
        text: output,
        status: status.code(),
    })
}

fn wait(child: &mut Child, stdout: &Path, stderr: &Path) -> Result<(ExitStatus, bool)> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok((status, false));
        }
        if started.elapsed() >= Duration::from_secs(600)
            || fs::metadata(stdout)?.len() > 16 * 1024 * 1024
            || fs::metadata(stderr)?.len() > 16 * 1024 * 1024
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
