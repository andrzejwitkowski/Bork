use bork::{dump::dump_arenas, parse, sema::analyze};
use std::env;
use std::fs;
use std::process;

fn usage() -> ! {
    eprintln!("Usage: bork [--dump-arenas] <file.bork>");
    process::exit(2);
}

fn main() {
    let mut dump = false;
    let mut file: Option<String> = None;

    for arg in env::args().skip(1) {
        match arg.as_str() {
            "--dump-arenas" => dump = true,
            "-h" | "--help" => usage(),
            other if other.starts_with('-') => {
                eprintln!("unknown flag: {other}");
                usage();
            }
            other => {
                if file.is_some() {
                    eprintln!("unexpected argument: {other}");
                    usage();
                }
                file = Some(other.to_string());
            }
        }
    }

    let Some(path) = file else {
        usage();
    };

    let source = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("failed to read {path}: {err}");
            process::exit(1);
        }
    };

    let program = match parse(&source) {
        Ok(p) => p,
        Err(err) => {
            eprintln!("parse error: {err}");
            process::exit(1);
        }
    };

    let (report, errors) = analyze(&program);
    for err in &errors {
        eprintln!("error: {}", err.message);
    }

    if dump {
        print!("{}", dump_arenas(&report));
    }

    if !errors.is_empty() {
        process::exit(1);
    }
}
