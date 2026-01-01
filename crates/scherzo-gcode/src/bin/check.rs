use std::{env, fs, path::Path};

fn main() {
    let args = env::args().skip(1);
    if args.len() == 0 {
        eprintln!("usage: check <file> [<file>...]");
        std::process::exit(1);
    }

    let mut failed = 0usize;
    for path in args {
        let path_ref = Path::new(&path);
        let input = match fs::read_to_string(path_ref) {
            Ok(s) => s,
            Err(err) => {
                eprintln!("{path}: read error: {err}");
                failed += 1;
                continue;
            }
        };

        match scherzo_gcode::parse(&input) {
            Ok(_) => {
                println!("OK {path}");
            }
            Err(err) => {
                println!("ERR {path}: {err}");
                failed += 1;
            }
        }
    }

    if failed > 0 {
        std::process::exit(1);
    }
}
