//! Bounded, static SVG-input policy for untrusted document images. Shares
//! usvg/resvg and the straight ARGB representation with the icon pipeline.

use super::{IconError, RgbaImage};
use alloc::{boxed::Box, vec::Vec};
use resvg::{tiny_skia, usvg};

/// Natural-size rasterization; CSS/HTML sizing remains the document's job.
/// No fonts, compressed SVG, DTDs, images, use expansion, markers, patterns,
/// masks or filters. No scripts, animation, foreignObject or external loads.
pub fn decode_document_svg(bytes: &[u8]) -> Result<RgbaImage, IconError> {
    if bytes.len() > 64 * 1024 {
        return Err(IconError::LimitExceeded);
    }
    let source = core::str::from_utf8(bytes).map_err(|_| IconError::InvalidSvg)?;
    let xml = usvg::roxmltree::Document::parse_with_options(
        source,
        usvg::roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 2048,
        },
    )
    .map_err(|_| IconError::InvalidSvg)?;
    for node in xml.descendants().filter(|n| n.is_element()) {
        if node.ancestors().take(34).count() > 32 {
            return Err(IconError::LimitExceeded);
        }
        if matches!(
            node.tag_name().name(),
            "use"
                | "image"
                | "filter"
                | "mask"
                | "pattern"
                | "marker"
                | "text"
                | "script"
                | "foreignObject"
                | "animate"
                | "animateMotion"
                | "animateTransform"
                | "set"
        ) {
            return Err(IconError::InvalidSvg);
        }
        if node
            .attributes()
            .any(|a| a.name() == "href" && !a.value().starts_with('#'))
        {
            return Err(IconError::InvalidSvg);
        }
    }
    let mut options = usvg::Options::default();
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    options.image_href_resolver.resolve_data = Box::new(|_, _, _| None);
    let tree = usvg::Tree::from_xmltree(&xml, &options).map_err(|_| IconError::InvalidSvg)?;
    let size = tree.size().to_int_size();
    let (width, height) = (size.width(), size.height());
    let count = (width as usize)
        .checked_mul(height as usize)
        .ok_or(IconError::LimitExceeded)?;
    if width > 1024 || height > 1024 || count > 256 * 1024 {
        return Err(IconError::LimitExceeded);
    }
    // Bound offscreen opacity/clipping layers as well as the <= 1 MiB raster.
    // Check layer extents ourselves, including geometry outside the viewBox.
    fn check(group: &usvg::Group, isolated: usize, nodes: &mut usize) -> Result<(), IconError> {
        let isolated = isolated + usize::from(group.should_isolate());
        if isolated > 2 {
            return Err(IconError::LimitExceeded);
        }
        if group.should_isolate() {
            let bounds = group.abs_layer_bounding_box();
            if bounds.x().abs() > 4096.0
                || bounds.y().abs() > 4096.0
                || bounds.width() > 1024.0
                || bounds.height() > 1024.0
                || bounds.width() * bounds.height() > (256 * 1024) as f32
            {
                return Err(IconError::LimitExceeded);
            }
        }
        for node in group.children() {
            *nodes += 1;
            if *nodes > 4096 {
                return Err(IconError::LimitExceeded);
            }
            if let usvg::Node::Group(g) = node {
                check(g, isolated, nodes)?;
            }
        }
        Ok(())
    }
    check(tree.root(), 0, &mut 0)?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(IconError::LimitExceeded)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(
            width as f32 / tree.size().width(),
            height as f32 / tree.size().height(),
        ),
        &mut pixmap.as_mut(),
    );
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(count)
        .map_err(|_| IconError::LimitExceeded)?;
    for p in pixmap.data().chunks_exact(4) {
        let a = u32::from(p[3]);
        let straight = |c: u8| {
            if a == 0 {
                0
            } else {
                ((u32::from(c) * 255 + a / 2) / a).min(255)
            }
        };
        pixels.push((a << 24) | (straight(p[0]) << 16) | (straight(p[1]) << 8) | straight(p[2]));
    }
    Ok(RgbaImage {
        width,
        height,
        pixels,
    })
}
