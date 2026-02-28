fn main() {
    if let Err(err) = keba_home_api::app::run_service() {
        eprintln!("service startup failed: {err}");
        std::process::exit(1);
    }
}
