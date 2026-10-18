fn main() {
    if let Err(error) = onecad_ui::run() {
        eprintln!("onecad-ui: {error}");
        std::process::exit(1);
    }
}
