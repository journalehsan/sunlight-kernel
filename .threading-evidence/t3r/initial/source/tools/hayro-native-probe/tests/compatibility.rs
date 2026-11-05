use sunlight_hayro_native_probe::{open, Error, MAX_SOURCE_BYTES};

const SAMPLE: &[u8] = include_bytes!("../../../assets/documents/SunlightOS_PDF_Reader_Test.pdf");

#[test]
fn parser_reads_sample_pages_and_decodes_content_without_std_features() {
    for _ in 0..8 {
        let pdf = open(SAMPLE).unwrap();
        assert_eq!(pdf.pages().len(), 4);
        for page in pdf.pages().iter() {
            let (w, h) = page.render_dimensions();
            assert!((w - 595.276).abs() < 0.01);
            assert!((h - 841.890).abs() < 0.01);
            assert!(page.typed_operations().next().is_some());
        }
    }
}

#[test]
fn rejects_bad_and_oversized_input() {
    assert!(matches!(open(b"not a PDF"), Err(Error::InvalidPdf)));
    assert!(matches!(
        open(&vec![0; MAX_SOURCE_BYTES + 1]),
        Err(Error::SourceTooLarge)
    ));
}

#[cfg(feature = "raster")]
#[test]
fn raster_matches_canvas_pixel_contract() {
    let image = sunlight_hayro_native_probe::vector_fixture().unwrap();
    assert_eq!((image.width, image.height), (64, 48));
    assert_eq!(image.pixels.len(), 64 * 48);
    assert_eq!(image.pixels[0], 0xffff_ffff);
    assert_eq!(image.pixels[12 * 64 + 12], 0xffff_0000);
    assert_eq!(
        image.pixels.iter().filter(|&&p| p == 0xffff_0000).count(),
        24 * 16
    );
}
