use sunlight_fat::testimg::FatImageBuilder;
fn main() {
    let mut builder = FatImageBuilder::new(1);
    let users = builder.add_dir(builder.root(), "USERS");
    builder.add_dir(users, "1000");
    let mut image = builder.build();
    image[0] = 0xeb;
    image[71..79].copy_from_slice(b"SUNSTATE");
    std::fs::write("docs/audit/probes/python/results-2026-11-14-py7/fat-fixture.img", image).unwrap();
}
