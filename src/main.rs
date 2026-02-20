mod adapters;
mod app;
mod domain;

fn main() {
    if let Err(err) = app::run() {
        eprintln!("application startup failed: {err}");
        std::process::exit(1);
    }
}
