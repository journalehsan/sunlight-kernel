//! UI-owned image admission. Resource readiness and script/event state never
//! depend on this policy. No pixels or DOM references are retained here.
use alloc::{
    collections::{BTreeMap, BTreeSet},
    vec::Vec,
};
use sunlight_ui::{
    widgets::{DocumentScene, RenderObject, RenderObjectId, RenderObjectKind},
    Rect,
};

/// A small paint batch per UI tick, with visible objects ahead of nearby ones.
pub const IMAGE_PAINT_BATCH: usize = 4;

#[derive(Default)]
pub struct ImagePresentation {
    document_id: u64,
    admitted: BTreeSet<RenderObjectId>,
    identities: BTreeMap<RenderObjectId, usize>,
    last_view: Option<(u64, u64, Rect)>,
    more: bool,
}

pub fn paint_bounds(object: &RenderObject) -> Option<Rect> {
    object
        .clip_bounds
        .map_or(Some(object.bounds), |clip| object.bounds.intersect(clip))
}

/// 0 visible, 1 within one viewport in either direction, 2 distant/hidden.
pub fn priority(bounds: Option<Rect>, viewport: Rect) -> u8 {
    let Some(bounds) = bounds else {
        return 2;
    };
    if bounds.intersect(viewport).is_some() {
        return 0;
    }
    let near = Rect::new(
        viewport.x,
        viewport.y.saturating_sub(viewport.h as i32),
        viewport.w,
        viewport.h.saturating_mul(3),
    );
    if bounds.intersect(near).is_some() {
        1
    } else {
        2
    }
}

impl ImagePresentation {
    pub fn admitted(&self) -> &BTreeSet<RenderObjectId> {
        &self.admitted
    }

    /// Reconcile by stable object ID on every scene/view change. Replacing
    /// pixels or moving an admitted object needs no second resource/decode job;
    /// the canvas paints the current object's pixels at its current geometry.
    pub fn advance(&mut self, scene: &DocumentScene, revision: u64, viewport: Rect) -> bool {
        let view = (scene.document_id, revision, viewport);
        if self.last_view == Some(view) && !self.more {
            return false;
        }
        self.last_view = Some(view);
        if self.document_id != scene.document_id {
            self.document_id = scene.document_id;
            self.admitted.clear();
            self.identities.clear();
        }
        self.admitted.retain(|id| {
            scene.object(*id).is_some_and(|o| match &o.kind {
                RenderObjectKind::Image { image, .. } => {
                    self.identities.get(id) == Some(&(image.pixels.as_ptr() as usize))
                }
                _ => false,
            })
        });
        self.identities.retain(|id, _| self.admitted.contains(id));
        let mut ready: Vec<_> = scene
            .objects
            .iter()
            .filter(|o| {
                matches!(o.kind, RenderObjectKind::Image { .. }) && !self.admitted.contains(&o.id)
            })
            .map(|o| (priority(paint_bounds(o), viewport), o.paint_order, o.id))
            .filter(|(p, _, _)| *p < 2)
            .collect();
        ready.sort_by_key(|(p, order, _)| (*p, *order));
        // Admitting nearby objects prepares future scroll paints; it does not
        // invalidate pixels in the current viewport.
        let changed = ready
            .iter()
            .take(IMAGE_PAINT_BATCH)
            .any(|(p, _, _)| *p == 0);
        self.more = ready.len() > IMAGE_PAINT_BATCH;
        for (_, _, id) in ready.into_iter().take(IMAGE_PAINT_BATCH) {
            self.admitted.insert(id);
            if let Some(RenderObjectKind::Image { image, .. }) = scene.object(id).map(|o| &o.kind) {
                self.identities.insert(id, image.pixels.as_ptr() as usize);
            }
        }
        changed
    }

    pub fn visible_complete(&self, scene: &DocumentScene, viewport: Rect) -> bool {
        scene
            .objects
            .iter()
            .filter(|o| {
                matches!(o.kind, RenderObjectKind::Image { .. })
                    && priority(paint_bounds(o), viewport) == 0
            })
            .all(|o| self.admitted.contains(&o.id))
    }
}
