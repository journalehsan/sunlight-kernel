//! Small, allocation-free layout primitives.
//!
//! The toolkit historically accepts final widget rectangles directly.  That
//! remains the default through [`AxisSizing::Legacy`].  Widgets and future
//! containers can opt into the measure/arrange contract in this module one
//! axis at a time without requiring a retained widget tree.

use crate::geom::{Rect, Size};

pub use crate::geom::{GridColIter, GridRow, HBox, VBox};

/// A widget's sizing intent on one axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AxisSizing {
    /// Preserve the previously supplied geometry on this axis.
    #[default]
    Legacy,
    /// Request an explicit number of pixels.
    Fixed(u32),
    /// Request the widget's natural/content size.
    Hug,
    /// Consume space available from the parent.
    Fill,
    /// Consume a weighted share of a parent's remaining space.
    ///
    /// A zero weight is treated as one so distribution always makes progress.
    Flex(u16),
}

impl AxisSizing {
    #[inline]
    const fn flex_weight(self) -> Option<u32> {
        match self {
            Self::Fill => Some(1),
            Self::Flex(weight) => Some(if weight == 0 { 1 } else { weight as u32 }),
            _ => None,
        }
    }
}

/// Independent horizontal and vertical sizing policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sizing {
    pub width: AxisSizing,
    pub height: AxisSizing,
}

impl Sizing {
    pub const LEGACY: Self = Self::new(AxisSizing::Legacy, AxisSizing::Legacy);

    #[inline]
    pub const fn new(width: AxisSizing, height: AxisSizing) -> Self {
        Self { width, height }
    }
}

/// Constraints supplied while measuring a widget.
///
/// `available` is the space offered by the parent.  It is deliberately
/// separate from `max`: fixed and legacy widgets may be larger than the space
/// offered and are then clipped as before, unless an explicit maximum says
/// otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutConstraints {
    pub min: Size,
    pub max: Size,
    pub available: Size,
}

impl LayoutConstraints {
    #[inline]
    pub const fn new(available: Size) -> Self {
        Self {
            min: Size::new(0, 0),
            max: Size::new(u32::MAX, u32::MAX),
            available,
        }
    }

    #[inline]
    pub const fn with_min(mut self, min: Size) -> Self {
        self.min = min;
        self
    }

    #[inline]
    pub const fn with_max(mut self, max: Size) -> Self {
        self.max = max;
        self
    }

    /// Clamp a size to normalized constraints.  If a caller supplies
    /// `min > max`, the explicit maximum wins rather than panicking.
    #[inline]
    pub fn constrain(self, size: Size) -> Size {
        Size::new(
            constrain_axis(size.w, self.min.w, self.max.w),
            constrain_axis(size.h, self.min.h, self.max.h),
        )
    }
}

#[inline]
const fn constrain_axis(value: u32, min: u32, max: u32) -> u32 {
    let normalized_min = if min > max { max } else { min };
    if value < normalized_min {
        normalized_min
    } else if value > max {
        max
    } else {
        value
    }
}

/// Measurement half of the layout contract.
pub trait Measure {
    /// Return the preferred size under the supplied constraints.
    fn measure(&self, constraints: LayoutConstraints) -> Size;
}

/// Final-assignment half of the layout contract.
pub trait Arrange {
    fn bounds(&self) -> Rect;
    fn set_bounds(&mut self, bounds: Rect);
}

/// Compatibility layout state for rectangle-based widgets.
///
/// Existing widgets can keep their public `Rect`; a future container can hold
/// this small value beside a widget, call [`Measure::measure`], then copy the
/// assigned [`LayoutBox::bounds`] into the widget before drawing.  This avoids
/// adding policy fields to every widget during Phase 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutBox {
    bounds: Rect,
    natural: Size,
    sizing: Sizing,
    min: Size,
    max: Size,
}

impl LayoutBox {
    /// Create a legacy box that preserves `bounds` exactly by default.
    pub const fn new(bounds: Rect) -> Self {
        Self {
            bounds,
            natural: Size::new(bounds.w, bounds.h),
            sizing: Sizing::LEGACY,
            min: Size::new(0, 0),
            max: Size::new(u32::MAX, u32::MAX),
        }
    }

    pub const fn with_sizing(mut self, sizing: Sizing) -> Self {
        self.sizing = sizing;
        self
    }

    pub const fn with_natural_size(mut self, natural: Size) -> Self {
        self.natural = natural;
        self
    }

    pub const fn with_min_size(mut self, min: Size) -> Self {
        self.min = min;
        self
    }

    pub const fn with_max_size(mut self, max: Size) -> Self {
        self.max = max;
        self
    }

    pub const fn sizing(&self) -> Sizing {
        self.sizing
    }

    pub const fn natural_size(&self) -> Size {
        self.natural
    }

    pub const fn bounds(&self) -> Rect {
        self.bounds
    }

    pub fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
    }

    /// Measure and assign this box inside `available`, returning final bounds.
    /// Legacy axes retain both their old origin and extent; opted-in axes use
    /// the corresponding origin supplied by the parent.
    pub fn layout(&mut self, available: Rect) -> Rect {
        let measured = self.measure(LayoutConstraints::new(available.size()));
        self.bounds = Rect::new(
            if self.sizing.width == AxisSizing::Legacy {
                self.bounds.x
            } else {
                available.x
            },
            if self.sizing.height == AxisSizing::Legacy {
                self.bounds.y
            } else {
                available.y
            },
            measured.w,
            measured.h,
        );
        self.bounds
    }
}

impl Measure for LayoutBox {
    fn measure(&self, constraints: LayoutConstraints) -> Size {
        let constraints = LayoutConstraints {
            min: Size::new(
                constraints.min.w.max(self.min.w),
                constraints.min.h.max(self.min.h),
            ),
            max: Size::new(
                constraints.max.w.min(self.max.w),
                constraints.max.h.min(self.max.h),
            ),
            available: constraints.available,
        };
        let requested = Size::new(
            measure_axis(
                self.sizing.width,
                self.bounds.w,
                self.natural.w,
                constraints.available.w,
            ),
            measure_axis(
                self.sizing.height,
                self.bounds.h,
                self.natural.h,
                constraints.available.h,
            ),
        );
        constraints.constrain(requested)
    }
}

impl Arrange for LayoutBox {
    fn bounds(&self) -> Rect {
        LayoutBox::bounds(self)
    }

    fn set_bounds(&mut self, bounds: Rect) {
        LayoutBox::set_bounds(self, bounds);
    }
}

#[inline]
const fn measure_axis(sizing: AxisSizing, legacy: u32, natural: u32, available: u32) -> u32 {
    match sizing {
        AxisSizing::Legacy => legacy,
        AxisSizing::Fixed(value) => value,
        AxisSizing::Hug => natural,
        AxisSizing::Fill | AxisSizing::Flex(_) => available,
    }
}

/// One child's requirements for allocation along a container's main axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisRequirement {
    pub sizing: AxisSizing,
    pub legacy: u32,
    pub natural: u32,
    pub min: u32,
    pub max: u32,
}

impl AxisRequirement {
    pub const fn new(sizing: AxisSizing, natural: u32) -> Self {
        Self {
            sizing,
            legacy: natural,
            natural,
            min: 0,
            max: u32::MAX,
        }
    }

    pub const fn legacy(size: u32) -> Self {
        Self::new(AxisSizing::Legacy, size)
    }

    pub const fn with_min(mut self, min: u32) -> Self {
        self.min = min;
        self
    }

    pub const fn with_max(mut self, max: u32) -> Self {
        self.max = max;
        self
    }
}

/// Errors from allocation-free axis distribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
    OutputTooSmall,
    TooManyChildren,
}

/// Resolve fixed/natural requirements, then divide remaining pixels among
/// `Fill`/`Flex` children without allocation.
///
/// Rounding is stable in input order. Explicit min/max values are honored;
/// impossible minimum totals may overflow the offered extent but arithmetic
/// never wraps.  This is the distribution primitive intended for Phase 2's
/// Row and Column containers.
pub fn distribute_axis(
    available: u32,
    gap: u32,
    requirements: &[AxisRequirement],
    output: &mut [u32],
) -> Result<(), LayoutError> {
    if output.len() < requirements.len() {
        return Err(LayoutError::OutputTooSmall);
    }

    let gap_count = u32::try_from(requirements.len().saturating_sub(1)).unwrap_or(u32::MAX);
    let gap_total = gap.saturating_mul(gap_count);
    let mut used = gap_total;
    for (index, requirement) in requirements.iter().enumerate() {
        let base = match requirement.sizing {
            AxisSizing::Legacy => requirement.legacy,
            AxisSizing::Fixed(value) => value,
            AxisSizing::Hug => requirement.natural,
            AxisSizing::Fill | AxisSizing::Flex(_) => requirement.min,
        };
        output[index] = constrain_axis(base, requirement.min, requirement.max);
        used = used.saturating_add(output[index]);
    }

    let mut remaining = available.saturating_sub(used);
    while remaining > 0 {
        let total_weight = requirements
            .iter()
            .enumerate()
            .filter(|(index, requirement)| {
                requirement.sizing.flex_weight().is_some() && output[*index] < requirement.max
            })
            .fold(0u64, |sum, (_, requirement)| {
                sum.saturating_add(requirement.sizing.flex_weight().unwrap_or(0) as u64)
            });
        if total_weight == 0 {
            break;
        }

        let round = remaining;
        let mut cumulative_weight = 0u64;
        let mut previous_target = 0u64;
        let mut distributed = 0u32;
        for (index, requirement) in requirements.iter().enumerate() {
            let Some(weight) = requirement.sizing.flex_weight() else {
                continue;
            };
            if output[index] >= requirement.max {
                continue;
            }
            cumulative_weight = cumulative_weight.saturating_add(weight as u64);
            let target = (round as u64).saturating_mul(cumulative_weight) / total_weight;
            let share = target.saturating_sub(previous_target).min(u32::MAX as u64) as u32;
            previous_target = target;
            let capacity = requirement.max.saturating_sub(output[index]);
            let granted = share.min(capacity);
            output[index] = output[index].saturating_add(granted);
            distributed = distributed.saturating_add(granted);
        }
        if distributed == 0 {
            break;
        }
        remaining = remaining.saturating_sub(distributed);
    }

    Ok(())
}

/// Tracks whether a layout root needs another deterministic layout pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutInvalidation {
    last_available: Option<Rect>,
    dirty: bool,
}

impl LayoutInvalidation {
    pub const fn new() -> Self {
        Self {
            last_available: None,
            dirty: true,
        }
    }

    /// Record current client bounds. Returns true when geometry changed or the
    /// layout was explicitly invalidated.
    pub fn update(&mut self, available: Rect) -> bool {
        let changed = self.last_available != Some(available);
        self.last_available = Some(available);
        let needs_layout = self.dirty || changed;
        self.dirty = false;
        needs_layout
    }

    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    pub const fn last_available(&self) -> Option<Rect> {
        self.last_available
    }
}

impl Default for LayoutInvalidation {
    fn default() -> Self {
        Self::new()
    }
}

/// Maximum child count handled by the stack-backed Row/Column scratch space.
/// Keeping this explicit makes layout time and stack use bounded in `no_std`
/// applications while comfortably covering ordinary application chrome.
pub const MAX_LINEAR_CHILDREN: usize = 64;

/// Uniform inset used by the small native linear containers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayoutPadding(pub u32);

impl LayoutPadding {
    pub const fn all(value: u32) -> Self {
        Self(value)
    }
}

fn inset_rect(rect: Rect, padding: LayoutPadding) -> Rect {
    let doubled = padding.0.saturating_mul(2);
    Rect::new(
        offset_i32(rect.x, padding.0),
        offset_i32(rect.y, padding.0),
        rect.w.saturating_sub(doubled),
        rect.h.saturating_sub(doubled),
    )
}

fn offset_i32(origin: i32, offset: u32) -> i32 {
    (i64::from(origin) + i64::from(offset)).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

impl LayoutBox {
    fn axis_requirement(&self, horizontal: bool) -> AxisRequirement {
        let (sizing, legacy, natural, min, max) = if horizontal {
            (
                self.sizing.width,
                self.bounds.w,
                self.natural.w,
                self.min.w,
                self.max.w,
            )
        } else {
            (
                self.sizing.height,
                self.bounds.h,
                self.natural.h,
                self.min.h,
                self.max.h,
            )
        };
        AxisRequirement {
            sizing,
            legacy,
            natural,
            min,
            max,
        }
    }
}

fn arrange_linear(
    bounds: Rect,
    padding: LayoutPadding,
    gap: u32,
    horizontal: bool,
    children: &mut [LayoutBox],
) -> Result<(), LayoutError> {
    if children.len() > MAX_LINEAR_CHILDREN {
        return Err(LayoutError::TooManyChildren);
    }
    let inner = inset_rect(bounds, padding);
    let mut requirements = [AxisRequirement::legacy(0); MAX_LINEAR_CHILDREN];
    let mut assigned = [0u32; MAX_LINEAR_CHILDREN];
    for (index, child) in children.iter().enumerate() {
        requirements[index] = child.axis_requirement(horizontal);
    }
    let main_available = if horizontal { inner.w } else { inner.h };
    distribute_axis(
        main_available,
        gap,
        &requirements[..children.len()],
        &mut assigned[..children.len()],
    )?;

    let child_count = children.len();
    let mut cursor = 0u32;
    for (index, child) in children.iter_mut().enumerate() {
        let measured = child.measure(LayoutConstraints::new(inner.size()));
        let rect = if horizontal {
            Rect::new(
                offset_i32(inner.x, cursor),
                inner.y,
                assigned[index],
                measured.h,
            )
        } else {
            Rect::new(
                inner.x,
                offset_i32(inner.y, cursor),
                measured.w,
                assigned[index],
            )
        };
        child.set_bounds(rect);
        cursor = cursor.saturating_add(assigned[index]);
        if index + 1 < child_count {
            cursor = cursor.saturating_add(gap);
        }
    }
    Ok(())
}

/// Allocation-free horizontal container for immediate-mode layout boxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    bounds: Rect,
    padding: LayoutPadding,
    gap: u32,
}

impl Row {
    pub const fn new(bounds: Rect) -> Self {
        Self {
            bounds,
            padding: LayoutPadding::all(0),
            gap: 0,
        }
    }

    pub const fn with_padding(mut self, padding: u32) -> Self {
        self.padding = LayoutPadding::all(padding);
        self
    }

    pub const fn with_gap(mut self, gap: u32) -> Self {
        self.gap = gap;
        self
    }

    pub fn arrange(self, children: &mut [LayoutBox]) -> Result<(), LayoutError> {
        arrange_linear(self.bounds, self.padding, self.gap, true, children)
    }
}

/// Allocation-free vertical container for immediate-mode layout boxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Column {
    bounds: Rect,
    padding: LayoutPadding,
    gap: u32,
}

impl Column {
    pub const fn new(bounds: Rect) -> Self {
        Self {
            bounds,
            padding: LayoutPadding::all(0),
            gap: 0,
        }
    }

    pub const fn with_padding(mut self, padding: u32) -> Self {
        self.padding = LayoutPadding::all(padding);
        self
    }

    pub const fn with_gap(mut self, gap: u32) -> Self {
        self.gap = gap;
        self
    }

    pub fn arrange(self, children: &mut [LayoutBox]) -> Result<(), LayoutError> {
        arrange_linear(self.bounds, self.padding, self.gap, false, children)
    }
}

/// Zero-natural-size layout participant. It has no drawing behavior; each
/// constructor merely expresses sizing intent through the common contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spacer {
    layout: LayoutBox,
}

impl Spacer {
    pub const fn horizontal() -> Self {
        Self {
            layout: LayoutBox::new(Rect::new(0, 0, 0, 0))
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Hug)),
        }
    }

    pub const fn vertical() -> Self {
        Self {
            layout: LayoutBox::new(Rect::new(0, 0, 0, 0))
                .with_sizing(Sizing::new(AxisSizing::Hug, AxisSizing::Fill)),
        }
    }

    pub const fn flexible() -> Self {
        Self {
            layout: LayoutBox::new(Rect::new(0, 0, 0, 0))
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fill)),
        }
    }
}

impl Measure for Spacer {
    fn measure(&self, constraints: LayoutConstraints) -> Size {
        self.layout.measure(constraints)
    }
}

impl Arrange for Spacer {
    fn bounds(&self) -> Rect {
        self.layout.bounds()
    }

    fn set_bounds(&mut self, bounds: Rect) {
        self.layout.set_bounds(bounds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_with(width: AxisSizing, height: AxisSizing) -> LayoutBox {
        LayoutBox::new(Rect::new(7, 9, 40, 20)).with_sizing(Sizing::new(width, height))
    }

    #[test]
    fn fixed_sizing_remains_fixed_across_parent_resize() {
        let mut layout = box_with(AxisSizing::Fixed(80), AxisSizing::Fixed(36));
        assert_eq!(
            layout.layout(Rect::new(0, 0, 200, 100)).size(),
            Size::new(80, 36)
        );
        assert_eq!(
            layout.layout(Rect::new(0, 0, 20, 10)).size(),
            Size::new(80, 36)
        );
    }

    #[test]
    fn fill_grows_and_shrinks_with_available_space() {
        let mut layout = box_with(AxisSizing::Fill, AxisSizing::Legacy);
        assert_eq!(layout.layout(Rect::new(0, 0, 200, 100)).w, 200);
        assert_eq!(layout.layout(Rect::new(0, 0, 12, 100)).w, 12);
    }

    #[test]
    fn axis_policies_are_independent() {
        let mut horizontal = box_with(AxisSizing::Fill, AxisSizing::Legacy);
        assert_eq!(
            horizontal.layout(Rect::new(0, 0, 90, 70)).size(),
            Size::new(90, 20)
        );

        let mut vertical = box_with(AxisSizing::Legacy, AxisSizing::Fill);
        assert_eq!(
            vertical.layout(Rect::new(0, 0, 90, 70)).size(),
            Size::new(40, 70)
        );
    }

    #[test]
    fn fill_on_both_axes_tracks_parent() {
        let mut layout = box_with(AxisSizing::Fill, AxisSizing::Fill);
        assert_eq!(
            layout.layout(Rect::new(2, 3, 90, 70)),
            Rect::new(2, 3, 90, 70)
        );
        assert_eq!(
            layout.layout(Rect::new(4, 5, 30, 10)),
            Rect::new(4, 5, 30, 10)
        );
    }

    #[test]
    fn flex_weights_divide_remaining_space_deterministically() {
        let requirements = [
            AxisRequirement::new(AxisSizing::Fixed(20), 0),
            AxisRequirement::new(AxisSizing::Flex(1), 0),
            AxisRequirement::new(AxisSizing::Flex(2), 0),
        ];
        let mut output = [0; 3];
        distribute_axis(101, 1, &requirements, &mut output).unwrap();
        assert_eq!(output, [20, 26, 53]);
        let first = output;
        distribute_axis(101, 1, &requirements, &mut output).unwrap();
        assert_eq!(output, first);
    }

    #[test]
    fn min_and_max_constraints_are_respected() {
        let mut minimum =
            box_with(AxisSizing::Fill, AxisSizing::Fill).with_min_size(Size::new(30, 25));
        assert_eq!(
            minimum.layout(Rect::new(0, 0, 1, 2)).size(),
            Size::new(30, 25)
        );

        let mut maximum =
            box_with(AxisSizing::Fill, AxisSizing::Fill).with_max_size(Size::new(60, 50));
        assert_eq!(
            maximum.layout(Rect::new(0, 0, 500, 400)).size(),
            Size::new(60, 50)
        );
    }

    #[test]
    fn legacy_default_retains_previous_geometry() {
        let original = Rect::new(7, 9, 40, 20);
        let mut layout = LayoutBox::new(original);
        assert_eq!(layout.layout(Rect::new(100, 200, 900, 800)), original);
    }

    #[test]
    fn repeated_layout_is_stable() {
        let available = Rect::new(3, 4, 77, 55);
        let mut layout =
            box_with(AxisSizing::Fill, AxisSizing::Hug).with_natural_size(Size::new(12, 18));
        let first = layout.layout(available);
        assert_eq!(layout.layout(available), first);
        assert_eq!(layout.layout(available), first);
    }

    #[test]
    fn zero_and_tiny_dimensions_are_valid() {
        let mut layout = box_with(AxisSizing::Fill, AxisSizing::Fill);
        assert_eq!(
            layout.layout(Rect::new(i32::MAX, i32::MAX, 0, 0)).size(),
            Size::new(0, 0)
        );

        let requirements = [
            AxisRequirement::new(AxisSizing::Fill, 0),
            AxisRequirement::new(AxisSizing::Flex(0), 0),
        ];
        let mut output = [u32::MAX; 2];
        distribute_axis(1, u32::MAX, &requirements, &mut output).unwrap();
        assert_eq!(output, [0, 0]);
    }

    #[test]
    fn distribution_redistributes_space_after_maximum() {
        let requirements = [
            AxisRequirement::new(AxisSizing::Flex(1), 0).with_max(10),
            AxisRequirement::new(AxisSizing::Flex(1), 0),
        ];
        let mut output = [0; 2];
        distribute_axis(40, 0, &requirements, &mut output).unwrap();
        assert_eq!(output, [10, 30]);
    }

    #[test]
    fn invalidation_only_relayouts_on_change_or_explicit_request() {
        let bounds = Rect::new(0, 0, 100, 80);
        let mut state = LayoutInvalidation::new();
        assert!(state.update(bounds));
        assert!(!state.update(bounds));
        state.invalidate();
        assert!(state.update(bounds));
        assert!(state.update(Rect::new(0, 0, 101, 80)));
        assert!(!state.update(Rect::new(0, 0, 101, 80)));
    }

    #[test]
    fn row_fill_consumes_horizontal_remainder_with_exact_padding_and_gap() {
        let mut children = [
            LayoutBox::new(Rect::new(0, 0, 50, 10))
                .with_sizing(Sizing::new(AxisSizing::Fixed(50), AxisSizing::Fill)),
            LayoutBox::new(Rect::new(0, 0, 0, 10))
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fill)),
        ];
        Row::new(Rect::new(2, 3, 300, 100))
            .with_padding(10)
            .with_gap(5)
            .arrange(&mut children)
            .unwrap();
        assert_eq!(children[0].bounds(), Rect::new(12, 13, 50, 80));
        assert_eq!(children[1].bounds(), Rect::new(67, 13, 225, 80));
    }

    #[test]
    fn column_fill_consumes_vertical_remainder() {
        let mut children = [
            LayoutBox::new(Rect::new(0, 0, 10, 24))
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fixed(24))),
            LayoutBox::new(Rect::new(0, 0, 10, 0))
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fill)),
        ];
        Column::new(Rect::new(0, 0, 120, 90))
            .with_gap(6)
            .arrange(&mut children)
            .unwrap();
        assert_eq!(children[0].bounds(), Rect::new(0, 0, 120, 24));
        assert_eq!(children[1].bounds(), Rect::new(0, 30, 120, 60));
    }

    #[test]
    fn row_flex_weights_and_constraints_are_deterministic() {
        let mut children = [
            LayoutBox::new(Rect::new(0, 0, 0, 8))
                .with_sizing(Sizing::new(AxisSizing::Flex(1), AxisSizing::Hug))
                .with_min_size(Size::new(20, 0))
                .with_max_size(Size::new(80, u32::MAX)),
            LayoutBox::new(Rect::new(0, 0, 0, 8))
                .with_sizing(Sizing::new(AxisSizing::Flex(2), AxisSizing::Hug))
                .with_min_size(Size::new(30, 0)),
        ];
        let row = Row::new(Rect::new(0, 0, 230, 20));
        row.arrange(&mut children).unwrap();
        assert_eq!(children[0].bounds().w, 80);
        assert_eq!(children[1].bounds().w, 150);
        let first = [children[0].bounds(), children[1].bounds()];
        row.arrange(&mut children).unwrap();
        assert_eq!([children[0].bounds(), children[1].bounds()], first);

        Row::new(Rect::new(0, 0, 10, 20))
            .arrange(&mut children)
            .unwrap();
        assert_eq!(children[0].bounds().w, 20);
        assert_eq!(children[1].bounds().w, 30);
    }

    #[test]
    fn linear_layout_keeps_tiny_rectangles_valid_and_spacers_are_intent_only() {
        let mut child = [LayoutBox::new(Rect::new(0, 0, 0, 0))
            .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fill))];
        Row::new(Rect::new(i32::MAX, i32::MAX, 1, 1))
            .with_padding(u32::MAX)
            .with_gap(u32::MAX)
            .arrange(&mut child)
            .unwrap();
        assert_eq!(child[0].bounds().size(), Size::new(0, 0));

        assert_eq!(
            Spacer::horizontal().measure(LayoutConstraints::new(Size::new(17, 23))),
            Size::new(17, 0)
        );
        assert_eq!(
            Spacer::vertical().measure(LayoutConstraints::new(Size::new(17, 23))),
            Size::new(0, 23)
        );
        assert_eq!(
            Spacer::flexible().measure(LayoutConstraints::new(Size::new(17, 23))),
            Size::new(17, 23)
        );
    }
}
