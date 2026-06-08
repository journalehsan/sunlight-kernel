use std::env;
use std::path::Path;

pub enum BuiltinResult {
    /// Builtin ran; exit code.
    Done(i32),
    /// Not a builtin — caller should exec externally.
    NotBuiltin,
    /// Shell should exit with this code.
    Exit(i32),
}

pub fn run(argv: &[&str]) -> BuiltinResult {
    match argv[0] {
        "exit" => BuiltinResult::Exit(0),
        "help" => {
            println!("Built-in commands: help, exit, cd, pwd, echo");
            BuiltinResult::Done(0)
        }
        "pwd" => match env::current_dir() {
            Ok(p) => {
                println!("{}", p.display());
                BuiltinResult::Done(0)
            }
            Err(e) => {
                eprintln!("sshl: pwd: {e}");
                BuiltinResult::Done(1)
            }
        },
        "echo" => {
            let out = argv[1..].join(" ");
            println!("{out}");
            BuiltinResult::Done(0)
        }
        "cd" => {
            let target = match argv.get(1) {
                Some(p) => p.to_string(),
                None => {
                    eprintln!("sshl: cd: missing argument");
                    return BuiltinResult::Done(1);
                }
            };
            if let Err(e) = env::set_current_dir(Path::new(&target)) {
                eprintln!("sshl: cd: {target}: {e}");
                BuiltinResult::Done(1)
            } else {
                BuiltinResult::Done(0)
            }
        }
        _ => BuiltinResult::NotBuiltin,
    }
}

