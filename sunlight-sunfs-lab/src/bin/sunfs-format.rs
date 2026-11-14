use std::path::Path;
use sunlight_sunfs::{
    bootstrap::Bootstrap,
    geometry::{Geometry, Parameters},
};
use sunlight_sunfs_lab::image::{parse_size, random_uuid, Image};
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 5
        || args[0] != "--image"
        || args[2] != "--size"
        || args[4] != "--confirm-disposable"
    {
        return Err(
            "usage: sunfs-format --image NEW_PATH --size INTEGER[MiB|GiB] --confirm-disposable"
                .into(),
        );
    }
    let size = parse_size(&args[3])?;
    // Complete planning precedes file creation.
    let geometry = Geometry::calculate(Parameters::laboratory(size, 256))
        .map_err(|e| format!("geometry: {e:?}"))?;
    let plan = Bootstrap::plan(geometry, random_uuid().map_err(|e| e.to_string())?)
        .map_err(|e| format!("plan: {e:?}"))?;
    let mut image = Image::create(Path::new(&args[1]), size, true).map_err(|e| e.to_string())?;
    plan.publish(&mut image)
        .map_err(|e| format!("format stopped; image incomplete/indeterminate: {e:?}"))?;
    println!("formatted disposable empty SunFS 0.2: {} blocks, {} metadata blocks, {} cleaner reserve blocks",geometry.volume_blocks,geometry.bootstrap_blocks,geometry.cleaner_reserve);
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(2);
    }
}
