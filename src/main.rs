use bork::{dump::dump_arenas, frontend, parse, sema::analyze};
use std::env;
use std::fs;
use std::process;

fn usage(status: i32) -> ! {
    let msg = "Usage: bork [check] [--dump-arenas] <file.bork>";
    if status == 0 {
        println!("{msg}");
    } else {
        eprintln!("{msg}");
    }
    process::exit(status);
}

fn main() {
    let mut check_cmd = false;
    let mut dump = false;
    let mut file: Option<String> = None;

    for arg in env::args().skip(1) {
        match arg.as_str() {
            "check" if !check_cmd => check_cmd = true,
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
    let _ = check_cmd;

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

    let check_res = frontend::check(&source);

    if let Err(ref diags) = check_res {
        for diag in diags {
            eprintln!("error: {:?}: {}", diag.phase, diag.message);
        }
    }

    if dump {
        if let Ok(program) = parse(&source) {
            let (report, _) = analyze(&program);
            print!("{}", dump_arenas(&report));
        }
    }

    if check_res.is_err() {
        process::exit(1);
    }
}
