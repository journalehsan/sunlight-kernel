use sunlight_hayro_prototype::{into_raster, Document, HayroDocument};

fn sample() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/documents/SunlightOS_PDF_Reader_Test.pdf"
    ))
    .unwrap()
}

#[test]
fn supplied_sample_dimensions_pixels_and_lifecycle() {
    for _ in 0..3 {
        let document = HayroDocument::load(sample()).unwrap();
        assert_eq!(document.page_count(), 4);
        for page in 0..4 {
            let info = document.page_info(page).unwrap();
            assert!((info.width_pt - 595.276).abs() < 0.01);
            assert!((info.height_pt - 841.890).abs() < 0.01);
            let bitmap = document.render_page(page, 1.0).unwrap();
            assert_eq!((bitmap.width(), bitmap.height()), (595, 841));
            assert_eq!(bitmap.data_as_u8_slice().len(), 595 * 841 * 4);
            let image = into_raster(bitmap).unwrap();
            assert!(
                image
                    .pixels
                    .iter()
                    .filter(|&&p| p & 0xffffff != 0xffffff)
                    .count()
                    > 1000
            );
            if page == 2 {
                // Vector curve, vector rounded rectangle, and cyan bitmap gradient.
                let colors = |predicate: fn(u8, u8, u8) -> bool| {
                    image
                        .pixels
                        .iter()
                        .filter(|&&p| predicate((p >> 16) as u8, (p >> 8) as u8, p as u8))
                        .count()
                };
                assert!(colors(|r, g, b| r > 180 && g < 150 && b < 150) > 500);
                assert!(colors(|r, g, b| r > 220 && g > 140 && g < 210 && b < 130) > 5000);
                assert!(colors(|r, g, b| r < 50 && g > 130 && b > 160) > 15000);
            }
        }
    }
}

#[test]
fn malformed_and_invalid_requests_are_errors() {
    for bytes in [vec![], b"not a PDF".to_vec(), b"%PDF-1.7\n%%EOF".to_vec()] {
        assert!(HayroDocument::load(bytes).is_err());
    }
    let document = HayroDocument::load(sample()).unwrap();
    assert!(document.page_info(4).is_err());
    assert!(document.render_page(usize::MAX, 1.0).is_err());
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, 4.1, 0.0001] {
        assert!(document.render_page(0, scale).is_err());
    }
}

#[test]
fn intrinsic_rotation_swaps_dimensions_and_rotates_artwork() {
    // Original fixture: red rectangle in the bottom-left of a 200x100 page.
    let stream = "1 0 0 rg 0 0 40 20 re f\n";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Rotate 90 /Contents 4 0 R >>"
            .to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{}endstream",
            stream.len(),
            stream
        ),
    ];
    let mut pdf = "%PDF-1.4\n".to_owned();
    let mut offsets = vec![0];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{}\nendobj\n", index + 1, object));
    }
    let xref = pdf.len();
    pdf.push_str("xref\n0 5\n0000000000 65535 f \n");
    for offset in &offsets[1..] {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
    ));
    let document = HayroDocument::load(pdf.into_bytes()).unwrap();
    let info = document.page_info(0).unwrap();
    assert_eq!((info.width_pt, info.height_pt), (100.0, 200.0));
    let image = into_raster(document.render_page(0, 1.0).unwrap()).unwrap();
    assert_eq!((image.width, image.height), (100, 200));
    assert_eq!(image.pixels[10 * 100 + 10], 0xffff0000);
    assert_eq!(image.pixels[100 * 100 + 50], 0xffffffff);
}
