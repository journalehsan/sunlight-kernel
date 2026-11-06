#![cfg(feature = "svg")]

use sunlight_ui::image::{decode_document_svg, IconError};

#[test]
fn static_svg_uses_natural_dimensions_and_straight_alpha() {
    let image = decode_document_svg(include_bytes!(
        "../../rappid-rabbit/tests/fixtures/viewbox.svg"
    ))
    .unwrap();
    assert_eq!((image.width, image.height), (80, 40));
    assert_eq!(image.pixels[0], 0);
    assert_eq!(image.pixels[20 * 80 + 40], 0x80ff0000);
}

#[test]
fn untrusted_svg_limits_source_raster_and_reference_expansion() {
    assert!(matches!(
        decode_document_svg(&vec![b' '; 65537]),
        Err(IconError::LimitExceeded)
    ));
    assert!(matches!(
        decode_document_svg(include_bytes!(
            "../../rappid-rabbit/tests/fixtures/oversized.svg"
        )),
        Err(IconError::LimitExceeded)
    ));
    for content in [
        "<use href='#a'/>",
        "<filter id='f'/>",
        "<pattern id='p'/>",
        "<script>bad()</script>",
        "<foreignObject/>",
        "<animate/>",
        "<image href='data:image/svg+xml;base64,PHN2Zy8+'/>",
        "<image href='https://example.com/image.png'/>",
        "<image href='/etc/passwd'/>",
    ] {
        let source = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'>{content}</svg>"
        );
        assert!(decode_document_svg(source.as_bytes()).is_err(), "{content}");
    }
    let nodes = format!(
        "<svg xmlns='http://www.w3.org/2000/svg'>{}</svg>",
        "<g/>".repeat(2049)
    );
    assert!(decode_document_svg(nodes.as_bytes()).is_err());
    let deep = format!(
        "<svg xmlns='http://www.w3.org/2000/svg'>{}<path d='M0 0L1 1'/>{}</svg>",
        "<g>".repeat(33),
        "</g>".repeat(33)
    );
    assert!(decode_document_svg(deep.as_bytes()).is_err());
}
