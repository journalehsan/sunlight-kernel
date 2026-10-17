//! Exercise the actual shipped assets, not just synthetic images smaller than
//! the application load limit. Keep filesystem reads in host-only tests.
use crate::editor::{inspect_input, Editor, Geometry, Sampler};
use std::path::Path;
use sunlight_ui::image::decode_simg;

#[test]
fn every_bundled_sample_passes_preflight_decodes_and_edits() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/images/Samples");
    let mut count = 0;
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|s| s.to_str()) != Some("simg") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let size =
            inspect_input(&bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert_eq!(size, (1312, 816), "{}", path.display());
        let image = decode_simg(&bytes).unwrap();
        assert_eq!((image.width, image.height), size);
        assert_eq!(image.pixels.len(), size.0 as usize * size.1 as usize);
        let mut editor = Editor::new(size.0, size.1);
        let neutral = Sampler::new(editor.state);
        assert_eq!(neutral.pixel(&image, 0, 0), image.pixel(0, 0));
        editor.geometry(Geometry::RotateRight).unwrap();
        assert_eq!(editor.state.size, (816, 1312));
        let rotated = Sampler::new(editor.state);
        assert_eq!(rotated.pixel(&image, 0, 0), image.pixel(0, 815));
        editor.undo();
        assert!(!editor.dirty());
        count += 1;
    }
    assert_eq!(
        count, 8,
        "all eight shipped sample photos must be exercised"
    );
}
