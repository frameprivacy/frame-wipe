fn main() {
    if let Err(e) = framewipe_cli::run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
