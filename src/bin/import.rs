fn main() {
    if let Err(e) = logos_rs::importer::run() {
        eprintln!("import failed: {e}");
        std::process::exit(1);
    }
}
