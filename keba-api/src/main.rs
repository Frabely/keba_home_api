fn main() {
    if let Err(err) = keba_home_api::app::run_api() {
        eprintln!("api startup failed: {err}");
        std::process::exit(1);
    }
}
