use bork::diag::Diagnostic;
use bork::{dump::dump_arenas, frontend};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;

fn usage(status: i32) -> ! {
    let msg = "Usage: bork [--dump-arenas] <file.bork>\n       bork build [-o <path>] <file.bork>";
    if status == 0 {
        println!("{msg}");
    } else {
        eprintln!("{msg}");
    }
    process::exit(status);
}

fn main() {
    let mut args = env::args().skip(1).peekable();
    if args.peek().map(String::as_str) == Some("build") {
        args.next();
        build_command(args.collect());
    }

    let mut dump = false;
    let mut file: Option<String> = None;

    for arg in args {
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

    let source = read_source(&path);
    let result = frontend::check(&source);
    print_diagnostics(&path, &source, &result.diagnostics);

    if dump {
        if let Some(report) = &result.report {
            print!("{}", dump_arenas(report));
        }
    }

    if !result.is_ok() {
        process::exit(1);
    }
}

fn build_command(args: Vec<String>) -> ! {
    let mut output: Option<PathBuf> = None;
    let mut file: Option<String> = None;

    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" => match args.next() {
                Some(path) => output = Some(PathBuf::from(path)),
                None => {
                    eprintln!("-o requires a path");
                    usage(2);
                }
            },
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
    let output = output.unwrap_or_else(|| PathBuf::from(&path).with_extension(""));
    run_build(&path, &output)
}

#[cfg(feature = "codegen")]
fn run_build(path: &str, output: &std::path::Path) -> ! {
    use bork::codegen::{build, BuildError};

    let source = read_source(path);
    let result = frontend::check(&source);
    match build(&result, output) {
        Ok(()) => process::exit(0),
        Err(BuildError::Diagnostics(diagnostics)) => {
            print_diagnostics(path, &source, &diagnostics);
            process::exit(1);
        }
        Err(BuildError::Toolchain(message)) => {
            eprintln!("error: {message}");
            process::exit(2);
        }
    }
}

#[cfg(not(feature = "codegen"))]
fn run_build(_path: &str, _output: &std::path::Path) -> ! {
    eprintln!("error: `bork build` requires bork to be compiled with the `codegen` feature");
    process::exit(2);
}

fn read_source(path: &str) -> String {
    match fs::read_to_string(path) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("failed to read {path}: {err}");
            process::exit(1);
        }
    }
}

fn print_diagnostics(path: &str, source: &str, diagnostics: &[Diagnostic]) {
    for diag in diagnostics {
        match diag.span {
            Some(span) => {
                let end = span.start.min(source.len());
                let before = &source[..end];
                let line = before.matches('\n').count() + 1;
                let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
                eprintln!(
                    "{path}:{line}:{col}: error: {}: {}",
                    diag.phase, diag.message
                );
            }
            None => eprintln!("{path}: error: {}: {}", diag.phase, diag.message),
        }
    }
}
