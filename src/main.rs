use bork::{dump::dump_arenas, frontend};
use std::env;
use std::fs;
use std::process;

fn usage(status: i32) -> ! {
    let msg = "Usage: bork [--dump-arenas] <file.bork>";
    if status == 0 {
        println!("{msg}");
    } else {
        eprintln!("{msg}");
    }
    process::exit(status);
}

fn main() {
    let mut dump = false;
    let mut file: Option<String> = None;

    for arg in env::args().skip(1) {
        match arg.as_str() {
            "--dump-arenas" => dump = true,
            "-h" | "--help" => usage(0),
            other if other.starts_with('-') => {
                eprintln!("unknown flag: {other}");
                usage(2);
            }
            other => {
                if file.is_some() {
                    eprintln!("unexpected argument: {other}");
                    usage(2);
                }
                file = Some(other.to_string());
            }
        }
    }

    let Some(path) = file else {
        usage(2);
    };

    let source = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("failed to read {path}: {err}");
            process::exit(1);
        }
    };

    let result = frontend::check(&source);

    for diag in &result.diagnostics {
        match diag.span {
            Some(span) => {
                let end = span.start.min(source.len());
                let before = &source[..end];
                let line = before.matches('\n').count() + 1;
                let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
                eprintln!("{path}:{line}:{col}: error: {}: {}", diag.phase, diag.message);
            }
            None => eprintln!("{path}: error: {}: {}", diag.phase, diag.message),
        }
    }

    if dump {
        if let Some(report) = &result.report {
            print!("{}", dump_arenas(report));
        }
    }

    if !result.is_ok() {
        process::exit(1);
    }
}
