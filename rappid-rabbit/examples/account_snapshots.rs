//! Deterministic evidence from Rabbit's retained scene and DocumentCanvas.
use rappid_rabbit::{
    account_fixture::{AccountFixture, Measure},
    dom_fixture::FixtureKey,
};
use std::{fs, io::Write};
use sunlight_ui::{app::App, Canvas, Theme};
fn save(app: &mut AccountFixture, name: &str) {
    let mut pixels = vec![0; 640 * 700];
    app.view(
        &mut Canvas::new(&mut pixels, 640, 640, 700),
        &Theme::sunlight_light(),
    );
    let mut file = std::io::BufWriter::new(
        fs::File::create(format!("target/account-fixture/{name}.ppm")).unwrap(),
    );
    file.write_all(b"P6\n640 700\n255\n").unwrap();
    for pixel in pixels {
        file.write_all(&[(pixel >> 16) as u8, (pixel >> 8) as u8, pixel as u8])
            .unwrap();
    }
}
fn main() {
    fs::create_dir_all("target/account-fixture").unwrap();
    let mut app = AccountFixture::new().unwrap();
    app.session.stress_gc();
    save(&mut app, "initial");
    app.session
        .evaluate(&mut app.render, "form.requestSubmit();", &Measure)
        .unwrap();
    save(&mut app, "invalid");
    app.session
        .evaluate(
            &mut app.render,
            "email.value='demo@example.test';password.value='invented';password.focus();",
            &Measure,
        )
        .unwrap();
    app.session
        .key(&mut app.render, FixtureKey::Enter, &Measure)
        .unwrap();
    save(&mut app, "loading");
    app.session
        .complete(&mut app.render, true, &Measure)
        .unwrap();
    save(&mut app, "success");
    println!(
        "Account fixture: initial, invalid, loading, success snapshots in target/account-fixture"
    );
}
