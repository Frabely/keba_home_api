use std::path::Path;

use keba_home_api::adapters::db::{open_connection, run_migrations, schema_version};

fn main() {
    if let Err(error) = run() {
        eprintln!("failed to create test db: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut path = if cfg!(windows) {
        ".\\data\\keba_test.db".to_string()
    } else {
        "./data/keba_test.db".to_string()
    };
    let mut force = false;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--path" => {
                let Some(value) = args.get(index + 1) else {
                    return Err("--path requires a value".to_string());
                };
                path = value.clone();
                index += 2;
            }
            "--force" => {
                force = true;
                index += 1;
            }
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            other => {
                return Err(format!("unknown argument: {other}"));
            }
        }
    }

    let path_ref = Path::new(&path);
    if let Some(parent) = path_ref.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create parent directory: {error}"))?;
    }

    if force && path_ref.exists() {
        std::fs::remove_file(path_ref)
            .map_err(|error| format!("failed to remove existing db file: {error}"))?;
    }

    let mut connection = open_connection(&path).map_err(|error| error.to_string())?;
    run_migrations(&mut connection).map_err(|error| error.to_string())?;
    let version = schema_version(&connection).map_err(|error| error.to_string())?;

    println!("created/updated test db at: {path}");
    println!("schema version: {version}");
    Ok(())
}

fn print_help() {
    println!("create_test_db");
    println!();
    println!("Usage:");
    println!("  cargo run --bin create_test_db -- [--path <file>] [--force]");
    println!();
    println!("Options:");
    println!(
        "  --path <file>   target sqlite file (default: .\\\\data\\\\keba_test.db on Windows)"
    );
    println!("  --force         delete existing file before creating");
}
