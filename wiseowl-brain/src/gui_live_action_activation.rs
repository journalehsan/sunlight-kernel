//! Deterministic production-path checks for Wise Owl GUI Live Action v1.
//!
//! The gate deliberately exercises the real bounded planner together with the
//! already-established attestation and GUI-bridge boundaries.  It contains no
//! launcher, session authority, or readiness-source substitute: those remain
//! available only through their existing production adapters.

#[cfg(all(feature = "sunlightos", feature = "gui-live-action-activation-v1-test"))]
use crate::{
    BoundedActionPlanner, PlannerContext, PlannerInput, PlannerResult, RequestedBy, SessionId,
    SunlightPlannerRegistry,
};

/// Gate used only by the QEMU activation fixture.  Production builds do not
/// compile a fake authority, readiness source, launcher, or executor here.
#[cfg(all(feature = "sunlightos", feature = "gui-live-action-activation-v1-test"))]
pub fn run_deterministic_live_action_gate() -> bool {
    if !crate::trusted_session_readiness::run_deterministic_trust_gate()
        || !crate::gui_bridge::run_deterministic_bridge_gate()
    {
        return false;
    }

    let context = PlannerContext {
        runtime_snapshot_generation: 1,
        active_session_id: SessionId(7),
        now: 100,
    };
    let request = |id, locale, text| {
        PlannerInput::direct(
            id,
            4,
            SessionId(7),
            RequestedBy::User(9),
            locale,
            text,
            1,
            100,
        )
    };
    let mut planner: BoundedActionPlanner<SunlightPlannerRegistry, 96> =
        BoundedActionPlanner::new(SunlightPlannerRegistry);

    matches!(
        planner.plan(&request(1, "en", "hi"), context),
        PlannerResult::NoAction
    ) && matches!(
        planner.plan(&request(2, "en", "Open Calculator"), context),
        PlannerResult::Proposed(_)
    ) && matches!(
        planner.plan(&request(3, "fa", "ماشین حساب را باز کن"), context),
        PlannerResult::Proposed(_)
    ) && matches!(
        planner.plan(&request(4, "en", "Open Display Settings"), context),
        PlannerResult::Proposed(_)
    ) && matches!(
        planner.plan(&request(5, "fa", "تنظیمات نمایش را باز کن"), context),
        PlannerResult::Proposed(_)
    ) && matches!(
        planner.plan(&request(6, "en", "delete a file"), context),
        PlannerResult::Unsupported(_) | PlannerResult::Unknown
    ) && matches!(
        planner.plan(&request(7, "en", "Open settings"), context),
        PlannerResult::NeedsClarification(_)
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn production_placeholder_text_is_not_present_in_live_action_module() {
        assert!(!include_str!("bin_parts/wiseowl-braind-native-body.rs")
            .contains("action requests are not enabled yet"));
    }
}
