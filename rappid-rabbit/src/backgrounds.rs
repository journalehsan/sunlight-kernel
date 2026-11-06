//! Bounded single-layer CSS url() backgrounds using the shared image cache.
//! Padding-box origin, border-box clip, scroll attachment; no multiple layers,
//! CSS escapes, background-origin/clip overrides, or fixed/local attachment.
use crate::{
    css::{css_image_url, ComputedStyle, Property, StyleContext},
    layout::{BoxType, LayoutResult, SceneImageSource},
};
use alloc::{string::String, vec::Vec};
use sunlight_ui::{
    widgets::{
        DocumentNodeId, DocumentScene, PaintOrder, RenderObject, RenderObjectId, RenderObjectKind,
    },
    Rect,
};

const MAX_BACKGROUND_TILES: usize = 4096;

fn value(style: &ComputedStyle, property: Property, default: &str) -> String {
    style
        .value(&property)
        .map(|v| v.display())
        .unwrap_or_else(|| String::from(default))
}
fn length(raw: &str, reference: f32) -> Option<f32> {
    if let Some(v) = raw.strip_suffix('%') {
        Some(v.parse::<f32>().ok()? * reference / 100.0)
    } else {
        raw.trim_end_matches("px").parse::<f32>().ok()
    }
}
fn position(raw: &str, available: f32) -> f32 {
    match raw {
        "left" | "top" => 0.0,
        "right" | "bottom" => available,
        "center" => available * 0.5,
        _ => length(raw, available).unwrap_or(0.0),
    }
}
fn size(raw: &str, area: (f32, f32), natural: (f32, f32)) -> (f32, f32) {
    if matches!(raw, "contain" | "cover") {
        let x = area.0 / natural.0;
        let y = area.1 / natural.1;
        let scale = if raw == "contain" { x.min(y) } else { x.max(y) };
        return (natural.0 * scale, natural.1 * scale);
    }
    let mut parts = raw.split_ascii_whitespace();
    let w = parts.next().and_then(|p| length(p, area.0));
    let h = parts.next().and_then(|p| length(p, area.1));
    match (w, h) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w * natural.1 / natural.0),
        (None, Some(h)) => (h * natural.0 / natural.1, h),
        _ => natural,
    }
}
fn int_rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(
        x.floor() as i32,
        y.floor() as i32,
        ((x + w).ceil() - x.floor()).max(0.0) as u32,
        ((y + h).ceil() - y.floor()).max(0.0) as u32,
    )
}

pub(crate) fn paint<S: SceneImageSource + ?Sized>(
    scene: &mut DocumentScene,
    result: &LayoutResult<'_>,
    styles: &StyleContext,
    images: &S,
) {
    // The flattened IR is parent-before-child. Order each box's background,
    // border and content together so descendant backgrounds cover ancestors.
    for object in &mut scene.objects {
        object.paint_order.z_index = if object.id.0 == u64::MAX {
            -1
        } else if object.id.0 >> 24 & 31 >= 30 {
            i32::MAX
        } else {
            object.paint_order.document_order.min(i32::MAX as u32 - 1) as i32
        };
    }
    let mut tiles = Vec::new();
    for (order, item) in result.render_boxes.iter().enumerate() {
        if !matches!(item.node_type, BoxType::Block | BoxType::Inline) {
            continue;
        }
        let Some(style) = styles.style_for(item.node_id) else {
            continue;
        };
        if !matches!(
            value(style, Property::BackgroundAttachment, "scroll").as_str(),
            "scroll" | "initial" | "normal" | ""
        ) {
            continue;
        }
        let raw = value(style, Property::BackgroundImage, "none");
        let Some(url) = css_image_url(&raw) else {
            continue;
        };
        let Some(image) = images.decoded(url) else {
            continue;
        };
        let Some(geometry) = result
            .geometry
            .boxes
            .iter()
            .find(|g| g.dom_node == item.node_id)
        else {
            continue;
        };
        let area = geometry.padding_box;
        let clip = geometry.border_box;
        let (w, h) = size(
            &value(style, Property::BackgroundSize, "auto"),
            (area.width, area.height),
            (image.width as f32, image.height as f32),
        );
        if !w.is_finite() || !h.is_finite() || w < 1.0 || h < 1.0 || w > 16384.0 || h > 16384.0 {
            continue;
        }
        let x = area.x
            + position(
                &value(style, Property::BackgroundPositionX, "0%"),
                area.width - w,
            );
        let y = area.y
            + position(
                &value(style, Property::BackgroundPositionY, "0%"),
                area.height - h,
            );
        let repeat = value(style, Property::BackgroundRepeat, "repeat");
        let repeat_x = matches!(repeat.as_str(), "repeat" | "repeat-x" | "");
        let repeat_y = matches!(repeat.as_str(), "repeat" | "repeat-y" | "");
        let first_x = if repeat_x {
            x + ((clip.x - x) / w).floor() * w
        } else {
            x
        };
        let first_y = if repeat_y {
            y + ((clip.y - y) / h).floor() * h
        } else {
            y
        };
        let cols = if repeat_x {
            ((clip.x + clip.width - first_x) / w).ceil().max(0.0) as usize
        } else {
            1
        };
        let rows = if repeat_y {
            ((clip.y + clip.height - first_y) / h).ceil().max(0.0) as usize
        } else {
            1
        };
        // Bounded work and scene memory even for a 1px tile on a huge document.
        for row in 0..rows.min(MAX_BACKGROUND_TILES) {
            for col in 0..cols.min(MAX_BACKGROUND_TILES) {
                if tiles.len() >= MAX_BACKGROUND_TILES {
                    break;
                }
                tiles.push(RenderObject {
                    id: RenderObjectId((1u64 << 63) | tiles.len() as u64),
                    owner_node_id: DocumentNodeId(item.node_id as u64),
                    kind: RenderObjectKind::Image {
                        alt: String::new(),
                        image: image.clone(),
                        source_url: images.resolve(url).unwrap_or_else(|| String::from(url)),
                        intrinsic_width: image.width,
                        intrinsic_height: image.height,
                    },
                    bounds: int_rect(first_x + col as f32 * w, first_y + row as f32 * h, w, h),
                    clip_bounds: Some(int_rect(clip.x, clip.y, clip.width, clip.height)),
                    paint_order: PaintOrder {
                        z_index: order.min(i32::MAX as usize - 1) as i32,
                        phase: 1,
                        document_order: u32::MAX,
                        ..PaintOrder::default()
                    },
                    interaction: None,
                });
            }
            if tiles.len() >= MAX_BACKGROUND_TILES {
                break;
            }
        }
    }
    for tile in tiles {
        scene.push(tile);
    }
    scene.finalize();
}
