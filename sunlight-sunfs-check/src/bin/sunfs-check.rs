use std::path::Path;
use sunlight_sunfs_check::{check, ReadFile, Status};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 || args[1] != "--image" {
        eprintln!("usage: sunfs-check --image PATH (read only)");
        std::process::exit(4);
    }
    match ReadFile::open(Path::new(&args[2])) {
        Ok(mut r) => {
            let result = check(&mut r);
            println!(
                "{} selected={:?} union_blocks={}\n{:#?}",
                result.status.name(),
                result.selected,
                result.union_blocks,
                result
            );
            std::process::exit(result.status.exit_code());
        }
        Err(e) => {
            eprintln!("INDETERMINATE: {e}");
            std::process::exit(Status::Indeterminate.exit_code());
        }
    }
}
