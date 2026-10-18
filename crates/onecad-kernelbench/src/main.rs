use onecad_kernelbench::{cli, AppError};

fn main() {
    let code = match cli::run(std::env::args().skip(1).collect()) {
        Ok(code) => code,
        Err(AppError { code, message }) => {
            eprintln!("kernelbench: {message}");
            code
        }
    };
    std::process::exit(code);
}
