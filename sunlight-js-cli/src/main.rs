use sunlight_js::{Context, Error, ErrorKind, JsResult, Value};
#[cfg(target_os = "sunlightos")]
extern crate sunlight_libc;

fn context() -> JsResult<Context> {
    let mut ctx = Context::default();
    ctx.register_native("print", |_, _, args| {
        println!(
            "{}",
            args.iter()
                .map(Value::display)
                .collect::<Vec<_>>()
                .join(" ")
        );
        Ok(Value::Undefined)
    })?;
    ctx.register_native("assert", |_, _, args| {
        if args.first().is_some_and(Value::truthy) {
            Ok(Value::Undefined)
        } else {
            Err(Error::new(ErrorKind::Exception, "native assertion failed"))
        }
    })?;
    Ok(ctx)
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut ctx = context()?;
    let mut args = std::env::args().skip(1);
    let file = args
        .next()
        .ok_or("usage: sunjs FILE | -e SOURCE | --smoke")?;
    let (source, name) = if file == "-e" {
        (
            args.next().ok_or("-e requires source")?,
            "<command>".to_string(),
        )
    } else if file == "--smoke" {
        (
            include_str!("../../sunlight-js/examples/smoke.js").to_string(),
            "<smoke>".to_string(),
        )
    } else {
        (std::fs::read_to_string(&file)?, file)
    };
    let result = ctx.eval(&source, &name)?;
    if result != Value::Undefined {
        println!("{}", result.display());
    }
    ctx.close()?;
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
