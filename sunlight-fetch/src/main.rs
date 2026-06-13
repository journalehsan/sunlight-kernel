//! fetch — SunlightOS lightweight HTTP downloader
//!
//! Entry point: parse args and test CLI parsing

use std::env;

use sunlight_fetch::cli;
use sunlight_fetch::error::FetchError;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    match run(&args) {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("fetch: error: {e}");
            std::process::exit(1);
        }
    }
}

fn run(args: &[String]) -> Result<(), FetchError> {
    let config = cli::parse_args(args).map_err(|e| FetchError::InvalidArgs(e))?;

    if config.help {
        let mut buf = String::with_capacity(1024);
        cli::print_help(&mut buf);
        println!("{}", buf);
        return Ok(());
    }

    // For now, just show parsed config
    println!("fetch: URL: {}", config.url);
    println!("fetch: Method: {}", config.method.as_str());
    println!("fetch: Chunks: {}", config.chunks);
    if let Some(output) = &config.output {
        println!("fetch: Output: {}", output);
    }

    Ok(())
}
