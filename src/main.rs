use bork::{parse, MVP_SAMPLE};

fn main() {
    match parse(MVP_SAMPLE) {
        Ok(program) => {
            println!("Parsed OK");
            println!("{program:#?}");
        }
        Err(err) => {
            eprintln!("Parse error: {err}");
            std::process::exit(1);
        }
    }
}
