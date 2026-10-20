mod checks;
mod evidence;
mod hygiene;
mod icons;
mod release;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn run() -> Result {
    std::env::set_current_dir(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap(),
    )?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, args) = args
        .split_first()
        .ok_or("usage: cargo xtask COMMAND [ARGS]")?;
    match command.as_str() {
        "check-rust-layers" | "verify-modeling-coverage" | "verify-modeling-contracts" => {
            checks::run(command, args)
        }
        "native-host-gate" | "verify-render-captures" | "kernelbench-manifest" => {
            evidence::run(command, args)
        }
        "bake-cad-icons" => icons::run(args),
        "check-hygiene" => hygiene::run(args),
        "prepare-release-materials"
        | "verify-release-materials"
        | "worker-manifest"
        | "rehash-worker-manifest"
        | "target-directory"
        | "manifest-hash" => release::run(command, args),
        _ => Err(format!("unknown xtask command: {command}").into()),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(
            error
                .downcast_ref::<evidence::ExitError>()
                .map_or(1, |error| error.code),
        );
    }
}
