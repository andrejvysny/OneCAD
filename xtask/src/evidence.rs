mod captures;
mod host;
mod host_names;
mod kernelbench;
mod process;
#[cfg(test)]
mod tests;

pub(super) type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[derive(Debug)]
pub struct ExitError {
    pub code: i32,
    message: String,
}
impl std::fmt::Display for ExitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl std::error::Error for ExitError {}
fn fail(code: i32, message: impl Into<String>) -> Box<dyn std::error::Error> {
    Box::new(ExitError {
        code,
        message: message.into(),
    })
}
pub fn run(command: &str, args: &[String]) -> Result<()> {
    match command {
        "native-host-gate" => host::run(args),
        "verify-render-captures" => {
            let path = args
                .first()
                .ok_or("usage: verify-render-captures DIRECTORY")?;
            println!(
                "{}",
                serde_json::to_string_pretty(&captures::verify(std::path::Path::new(path))?)?
            );
            Ok(())
        }
        "kernelbench-manifest" => kernelbench::run(args),
        _ => Err(format!("unknown evidence command: {command}").into()),
    }
}
