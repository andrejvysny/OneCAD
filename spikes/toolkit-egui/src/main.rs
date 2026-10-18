fn main() {
    if let Err(e) = toolkit_egui::run() {
        eprintln!("toolkit-egui: {e}");
        std::process::exit(1);
    }
}
