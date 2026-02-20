fn main() {
    if let Err(err) = keba_home_api::app::run() {
        eprintln!("application startup failed: {err}");
        std::process::exit(1);
    }
}
