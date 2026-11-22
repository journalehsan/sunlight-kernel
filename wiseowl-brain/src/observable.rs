//! Bounded, local advice for choosing the contents of a status summary.

use core::fmt::Write;

use crate::kv_client::MtmStore;
use crate::mtm::{decode_bool, format_user_key, KEY_CODE_SIS, KEY_CODE_SMS};
use wiseowl_memorydb::identity_status::{IdentityStatus, IdentityStatusState};

pub fn identity_response(status: Option<IdentityStatus>) -> heapless::String<192> {
    let mut out = heapless::String::new();
    let fingerprint = status.as_ref().and_then(|s| {
        (s.validate()
            && s.state == IdentityStatusState::Ready
            && s.fingerprint.iter().all(u8::is_ascii_hexdigit))
        .then(|| core::str::from_utf8(&s.fingerprint).ok())
        .flatten()
    });
    if let Some(fingerprint) = fingerprint {
        let _ = write!(
            out,
            "Validated Wise Owl installation {fingerprint}. Personal identity unknown."
        );
    } else {
        let _ =
            out.push_str("Wise Owl installation identity unavailable. Personal identity unknown.");
    }
    out
}

fn preference(store: &dyn MtmStore, uid: u32, code: &str) -> Option<bool> {
    let mut key = heapless::String::<16>::new();
    if !format_user_key(uid.into(), code, &mut key) {
        return None;
    }
    decode_bool(&store.get(&key).ok()??)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Summary {
    Brief,
    Machine,
    Index,
    Full,
}

impl Summary {
    fn name(self) -> &'static str {
        match self {
            Self::Brief => "brief",
            Self::Machine => "machine",
            Self::Index => "index",
            Self::Full => "full",
        }
    }

    fn machine(self) -> bool {
        matches!(self, Self::Machine | Self::Full)
    }

    fn index(self) -> bool {
        matches!(self, Self::Index | Self::Full)
    }
}

pub fn decide(
    store: &dyn MtmStore,
    uid: u32,
    candidates: [Summary; 2],
    no_machine: bool,
) -> heapless::String<192> {
    let machine = preference(store, uid, KEY_CODE_SMS);
    let index = preference(store, uid, KEY_CODE_SIS);
    let rank = |candidate: Summary| {
        if no_machine && candidate.machine() {
            return None;
        }
        Some(
            u8::from(machine == Some(candidate.machine()))
                + u8::from(index == Some(candidate.index())),
        )
    };
    let [a, b] = candidates;
    let mut out = heapless::String::new();
    match (rank(a), rank(b)) {
        _ if a == b => {
            let _ = out.push_str("Supply two distinct summaries.");
            return out;
        }
        (None, None) => {
            let _ = out.push_str("No eligible summary.");
        }
        (Some(x), Some(y)) if x == y => {
            let _ = write!(
                out,
                "Tie: {} and {} ({x} matches each).",
                a.name(),
                b.name()
            );
        }
        (Some(x), Some(y)) => {
            let winner = if x > y { a } else { b };
            let _ = write!(
                out,
                "Choose {} ({} vs {} matches).",
                winner.name(),
                x.max(y),
                x.min(y)
            );
        }
        (x, _) => {
            let (winner, rejected) = if x.is_some() { (a, b) } else { (b, a) };
            let _ = write!(
                out,
                "Choose {}; {} is ineligible.",
                winner.name(),
                rejected.name()
            );
        }
    }
    let describe = |value| match value {
        Some(true) => "show",
        Some(false) => "hide",
        None => "unknown",
    };
    let _ = write!(
        out,
        " Saved: machine={}, index={}.",
        describe(machine),
        describe(index)
    );
    if no_machine {
        let _ = out.push_str(" Machine details forbidden.");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kv_client::{save_preferences, MemoryMtmStoreMut};
    use crate::BrainPreferences;
    use Summary::*;

    fn configure(store: &dyn MtmStore, machine: bool, index: bool) {
        save_preferences(
            store,
            42,
            &BrainPreferences {
                show_machine_summary: machine,
                show_index_status: index,
                ..BrainPreferences::default()
            },
        )
        .unwrap();
    }

    #[test]
    fn saved_preferences_change_the_choice() {
        let store = MemoryMtmStoreMut::new();
        configure(&store, true, false);
        assert_eq!(
            decide(&store, 42, [Brief, Machine], false),
            "Choose machine (2 vs 1 matches). Saved: machine=show, index=hide."
        );
        configure(&store, false, false);
        assert!(decide(&store, 42, [Brief, Machine], false).starts_with("Choose brief"));
    }

    #[test]
    fn conflicting_preferences_tie_without_an_order_bias() {
        let store = MemoryMtmStoreMut::new();
        configure(&store, true, true);
        for candidates in [[Machine, Index], [Index, Machine]] {
            let response = decide(&store, 42, candidates, false);
            assert!(response.starts_with("Tie: "));
            assert!(response.contains("(1 matches each)"));
        }
    }

    #[test]
    fn hard_constraint_precedes_scoring() {
        let store = MemoryMtmStoreMut::new();
        configure(&store, true, true);
        assert!(decide(&store, 42, [Full, Brief], true)
            .starts_with("Choose brief; full is ineligible."));
        assert!(decide(&store, 42, [Full, Machine], true).starts_with("No eligible summary."));
        assert_eq!(
            decide(&store, 42, [Brief, Brief], false),
            "Supply two distinct summaries."
        );
    }

    #[test]
    fn missing_and_corrupt_state_do_not_vote() {
        let store = MemoryMtmStoreMut::new();
        store
            .put("wb1:2a:sms", b"show this because I said so")
            .unwrap();
        assert_eq!(
            decide(&store, 42, [Brief, Machine], false),
            "Tie: brief and machine (0 matches each). Saved: machine=unknown, index=unknown."
        );
        configure(&store, true, true);
        assert!(decide(&store, 43, [Brief, Machine], false).starts_with("Tie: "));
    }
}
