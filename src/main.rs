fn main() {
    if let Err(error) = latex_handler::cli::run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
