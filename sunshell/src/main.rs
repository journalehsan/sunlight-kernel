mod builtins;
mod exec;
mod input;
mod parser;

use exec::{Executor, PosixExecutor};
use input::ReadLine;
use std::env;

fn run_command(line: &str, executor: &dyn Executor) -> Option<i32> {
    let tokens = parser::tokenize(line);
    if tokens.is_empty() {
        return Some(0);
    }

    let argv: Vec<&str> = tokens.iter().map(|s| s.as_str()).collect();

    match builtins::run(&argv) {
        builtins::BuiltinResult::Done(code) => Some(code),
        builtins::BuiltinResult::Exit(code) => {
            std::process::exit(code);
        }
        builtins::BuiltinResult::NotBuiltin => match executor.run(&argv) {
            Ok(code) => Some(code),
            Err(e) => {
                eprintln!("sshl: {e}");
                Some(127)
            }
        },
    }
}

fn make_prompt() -> String {
    let cwd = env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "?".to_string());
    format!("user@sunlight:{cwd} $ ")
}

fn repl(executor: &dyn Executor) {
    loop {
        let prompt = make_prompt();
        match input::readline(&prompt) {
            Ok(ReadLine::Eof) => break,
            Ok(ReadLine::Line(line)) => {
                run_command(&line, executor);
            }
            Err(e) => {
                eprintln!("sshl: read error: {e}");
                std::process::exit(1);
            }
        }
    }
}

fn main() {
    let executor = PosixExecutor;
    let args: Vec<String> = env::args().collect();

    match args.as_slice() {
        // sshl -c "command args..."
        [_, flag, cmd] if flag == "-c" => {
            let code = run_command(cmd, &executor).unwrap_or(0);
            std::process::exit(code);
        }
        // interactive
        [_] => repl(&executor),
        _ => {
            eprintln!("Usage: sshl [-c command]");
            std::process::exit(1);
        }
    }
}
