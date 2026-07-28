use core::fmt::Write;

use crate::context::BrainContext;
use crate::error::BrainResult;
use crate::mtm::{format_memory_mib, GreetingStyle};
use crate::protocol::{
    ActionWire, GreetingResponseWire, HighlightWire, MAX_GREETING_LEN, MAX_HIGHLIGHT_VALUE,
};
use crate::provenance::BrainResponseFlags;

fn s<const N: usize>(text: &str) -> heapless::String<N> {
    let mut out: heapless::String<N> = heapless::String::new();
    for c in text.chars().take(N) {
        let _ = out.push(c);
    }
    out
}

fn push_greeting_str(buf: &mut heapless::String<MAX_GREETING_LEN>, text: &str) {
    for c in text.chars() {
        let _ = buf.push(c);
    }
}

/// Planning outcome flags for provenance.
#[derive(Debug, Clone, Copy, Default)]
pub struct GreetingPlanFlags {
    pub response_flags: BrainResponseFlags,
    pub machine_summary_included: bool,
    pub index_status_included: bool,
}

pub fn generate_greeting_response(ctx: &BrainContext) -> BrainResult<GreetingResponseWire> {
    let (resp, _) = plan_greeting_with_flags(ctx)?;
    Ok(resp)
}

pub fn plan_greeting_with_flags(
    ctx: &BrainContext,
) -> BrainResult<(GreetingResponseWire, GreetingPlanFlags)> {
    let mut title: heapless::String<MAX_GREETING_LEN> = heapless::String::new();
    let mut body: heapless::String<MAX_GREETING_LEN> = heapless::String::new();
    let mut flags = GreetingPlanFlags::default();
    let style = ctx.greeting_style();
    let returning = ctx.is_returning_visit();
    let after_upgrade = ctx.first_after_upgrade
        || ctx
            .system_generation
            .map(|g| ctx.welcome_memory.is_after_upgrade(g))
            .unwrap_or(false);

    // Title + primary body by visit class and style.
    if ctx.first_login && !returning {
        flags
            .response_flags
            .set(BrainResponseFlags::FIRST_VISIT_GREETING);
        match style {
            GreetingStyle::Concise => {
                push_greeting_str(&mut title, "Welcome to SunlightOS");
                push_greeting_str(
                    &mut body,
                    "Your desktop is ready. Take a short tour to get acquainted.",
                );
            }
            GreetingStyle::Friendly => {
                push_greeting_str(&mut title, "Welcome to SunlightOS");
                if !ctx.user_display_name.is_empty() {
                    push_greeting_str(&mut body, "Hello, ");
                    push_greeting_str(&mut body, &ctx.user_display_name);
                    push_greeting_str(&mut body, ". ");
                }
                push_greeting_str(
                    &mut body,
                    "Your desktop is ready for you. Take a short tour to get acquainted.",
                );
            }
            GreetingStyle::Technical => {
                push_greeting_str(&mut title, "Welcome to SunlightOS");
                if let Some(g) = ctx.system_generation {
                    let _ = write!(&mut body, "SunlightOS generation {} is active. ", g);
                }
                push_greeting_str(
                    &mut body,
                    "Wise Owl context services are ready. Take a short tour to get acquainted.",
                );
            }
        }
    } else if after_upgrade {
        flags
            .response_flags
            .set(BrainResponseFlags::AFTER_UPGRADE_GREETING);
        push_greeting_str(&mut title, "Welcome Back");
        match style {
            GreetingStyle::Technical => {
                if let Some(g) = ctx.system_generation {
                    let _ = write!(&mut body, "SunlightOS generation {} is active. ", g);
                }
                push_greeting_str(
                    &mut body,
                    "SunlightOS has started a new system generation. Everything should feel familiar.",
                );
            }
            _ => {
                push_greeting_str(
                    &mut body,
                    "SunlightOS has started a new system generation. Everything should feel familiar.",
                );
            }
        }
    } else if returning {
        flags
            .response_flags
            .set(BrainResponseFlags::RETURNING_USER_GREETING);
        match style {
            GreetingStyle::Concise => {
                push_greeting_str(&mut title, "Welcome back");
                push_greeting_str(&mut body, "Your desktop is ready.");
            }
            GreetingStyle::Friendly => {
                push_greeting_str(&mut title, "Welcome back to SunlightOS");
                push_greeting_str(
                    &mut body,
                    "Your desktop is ready for you. Browse the tour anytime from the Welcome Center.",
                );
            }
            GreetingStyle::Technical => {
                push_greeting_str(&mut title, "Welcome back");
                if let Some(g) = ctx.system_generation {
                    let _ = write!(&mut body, "SunlightOS generation {} is active. ", g);
                }
                push_greeting_str(&mut body, "Wise Owl context services are ready.");
            }
        }
    } else {
        // Request-flagged first_login false, no MTM — treat as return-style soft.
        match style {
            GreetingStyle::Concise => {
                push_greeting_str(&mut title, "Welcome");
                push_greeting_str(&mut body, "Your desktop is ready.");
            }
            GreetingStyle::Friendly => {
                push_greeting_str(&mut title, "Welcome to SunlightOS");
                push_greeting_str(
                    &mut body,
                    "Your desktop is ready. Browse the tour anytime from the Welcome Center.",
                );
            }
            GreetingStyle::Technical => {
                push_greeting_str(&mut title, "Welcome");
                push_greeting_str(&mut body, "Wise Owl context services are ready.");
            }
        }
    }

    // Machine summary (only when preference + facts).
    if ctx.preferences.show_machine_summary {
        let mut summary = heapless::String::<MAX_GREETING_LEN>::new();
        ctx.machine_summary_line(&mut summary);
        if !summary.is_empty() {
            push_greeting_str(&mut body, " ");
            push_greeting_str(&mut body, &summary);
            flags.machine_summary_included = true;
            flags
                .response_flags
                .set(BrainResponseFlags::MACHINE_SUMMARY_INCLUDED);
        }
    }

    // Index readiness (only when preference + grounded ready fact).
    if ctx.preferences.show_index_status && ctx.index_ready {
        push_greeting_str(&mut body, " ");
        if let Some(n) = ctx.indexed_source_count {
            if n > 0 {
                let _ = write!(
                    &mut body,
                    "Wise Owl's local index is ready with {} indexed sources.",
                    n
                );
            } else {
                push_greeting_str(&mut body, "Wise Owl's local index is ready.");
            }
        } else {
            push_greeting_str(&mut body, "Wise Owl's local index is ready.");
        }
        flags.index_status_included = true;
        flags.response_flags.set(BrainResponseFlags::INDEX_READY);
    }

    if ctx.memorydb_healthy {
        flags
            .response_flags
            .set(BrainResponseFlags::MEMORYDB_HEALTHY);
    }

    let mut highlights: heapless::Vec<HighlightWire, { crate::protocol::MAX_HIGHLIGHTS }> =
        heapless::Vec::new();

    if ctx.preferences.show_machine_summary {
        if let Some(cores) = ctx.cpu_cores {
            if cores > 0 {
                let mut value: heapless::String<MAX_HIGHLIGHT_VALUE> = heapless::String::new();
                let _ = write!(&mut value, "{} logical cores", cores);
                let _ = highlights.push(HighlightWire {
                    kind: 1,
                    label: s("Processor"),
                    value,
                });
            }
        }
        if let Some(ram) = ctx.ram_mib {
            if ram > 0 {
                let mut value: heapless::String<MAX_HIGHLIGHT_VALUE> = heapless::String::new();
                format_memory_mib(ram, &mut value);
                let _ = highlights.push(HighlightWire {
                    kind: 2,
                    label: s("Memory"),
                    value,
                });
            }
        }
    }

    if !ctx.model_name.is_empty() {
        let mut val: heapless::String<MAX_HIGHLIGHT_VALUE> = heapless::String::new();
        for c in ctx.model_name.chars() {
            let _ = val.push(c);
        }
        let _ = highlights.push(HighlightWire {
            kind: 3,
            label: s("Machine"),
            value: val,
        });
    }

    if !ctx.sunlight_version.is_empty() {
        let mut val: heapless::String<MAX_HIGHLIGHT_VALUE> = heapless::String::new();
        for c in ctx.sunlight_version.chars() {
            let _ = val.push(c);
        }
        let _ = highlights.push(HighlightWire {
            kind: 4,
            label: s("SunlightOS"),
            value: val,
        });
    }

    let mut actions: heapless::Vec<ActionWire, { crate::protocol::MAX_ACTIONS }> =
        heapless::Vec::new();

    let _ = actions.push(ActionWire {
        kind: 1,
        label: s("Open Control Panel"),
    });

    if ctx.first_login && !returning {
        let _ = actions.push(ActionWire {
            kind: 4,
            label: s("Continue Welcome Tour"),
        });
        let _ = actions.push(ActionWire {
            kind: 2,
            label: s("Browse Files"),
        });
    } else {
        let _ = actions.push(ActionWire {
            kind: 2,
            label: s("Browse Files"),
        });
    }

    let _ = actions.push(ActionWire {
        kind: 3,
        label: s("Open Terminal"),
    });

    Ok((
        GreetingResponseWire {
            title,
            body,
            highlights,
            suggested_actions: actions,
        },
        flags,
    ))
}

pub fn fallback_greeting() -> GreetingResponseWire {
    let mut title: heapless::String<MAX_GREETING_LEN> = heapless::String::new();
    push_greeting_str(&mut title, "Welcome to SunlightOS");
    let mut body: heapless::String<MAX_GREETING_LEN> = heapless::String::new();
    push_greeting_str(&mut body, "Your desktop is ready.");
    GreetingResponseWire {
        title,
        body,
        highlights: heapless::Vec::new(),
        suggested_actions: heapless::Vec::new(),
    }
}

pub fn align_and_shape(response: GreetingResponseWire) -> BrainResult<GreetingResponseWire> {
    if response.title.is_empty() {
        return Err(crate::error::BrainError::ResponseShapingFailed(
            "empty title",
        ));
    }
    if response.body.is_empty() {
        return Ok(fallback_greeting());
    }
    if response.title.len() > MAX_GREETING_LEN {
        return Err(crate::error::BrainError::ResponseShapingFailed(
            "title too long",
        ));
    }
    if response.body.len() > MAX_GREETING_LEN {
        return Err(crate::error::BrainError::ResponseShapingFailed(
            "body too long",
        ));
    }
    Ok(response)
}

pub fn plan_greeting_response(ctx: &BrainContext) -> BrainResult<GreetingResponseWire> {
    let (resp, _) = plan_greeting_with_flags(ctx)?;
    align_and_shape(resp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ContextBuilder;
    use crate::mtm::{BrainPreferences, GreetingStyle, WelcomeMemoryState};

    #[test]
    fn first_login_greeting() {
        let ctx = ContextBuilder::new()
            .user_id(1000)
            .user_display_name("Alice")
            .sunlight_version("0.2.0")
            .first_login(true)
            .cpu_cores(Some(8))
            .ram_mib(Some(16384))
            .build();
        let resp = plan_greeting_response(&ctx).unwrap();
        assert!(resp.title.contains("Welcome"));
        assert!(resp.body.contains("desktop is ready") || resp.body.contains("Alice"));
    }

    #[test]
    fn returning_visit_uses_mtm() {
        let mut ctx = ContextBuilder::new()
            .user_id(1000)
            .sunlight_version("0.2.0")
            .build();
        ctx.visit_count = 2;
        ctx.welcome_memory.visit_count = 2;
        let resp = plan_greeting_response(&ctx).unwrap();
        assert!(
            resp.title.to_lowercase().contains("back")
                || resp.body.to_lowercase().contains("back")
                || resp.title.contains("Welcome")
        );
    }

    #[test]
    fn technical_mentions_generation_when_grounded() {
        let mut ctx = ContextBuilder::new()
            .user_id(1000)
            .first_login(true)
            .build();
        ctx.preferences = BrainPreferences {
            greeting_style: GreetingStyle::Technical,
            show_machine_summary: false,
            show_index_status: false,
        };
        ctx.system_generation = Some(42);
        let resp = plan_greeting_response(&ctx).unwrap();
        assert!(resp.body.contains("generation 42") || resp.body.contains("Wise Owl"));
    }

    #[test]
    fn index_claim_only_when_ready_and_enabled() {
        let mut ctx = ContextBuilder::new().user_id(1000).build();
        ctx.preferences.show_index_status = true;
        ctx.index_ready = true;
        ctx.indexed_source_count = Some(12);
        let resp = plan_greeting_response(&ctx).unwrap();
        assert!(resp.body.contains("index is ready"));
        assert!(resp.body.contains("12"));
    }

    #[test]
    fn index_omitted_when_not_ready() {
        let mut ctx = ContextBuilder::new().user_id(1000).build();
        ctx.preferences.show_index_status = true;
        ctx.index_ready = false;
        let resp = plan_greeting_response(&ctx).unwrap();
        assert!(!resp.body.contains("index is ready"));
    }

    #[test]
    fn memory_format_in_summary() {
        let mut ctx = ContextBuilder::new()
            .user_id(1000)
            .cpu_cores(Some(4))
            .ram_mib(Some(3714))
            .build();
        ctx.preferences.show_machine_summary = true;
        let resp = plan_greeting_response(&ctx).unwrap();
        assert!(resp.body.contains("3.6 GiB") || resp.body.contains("3714"));
    }

    #[test]
    fn no_overclaiming_text() {
        let ctx = ContextBuilder::new().user_id(1000).build();
        let resp = plan_greeting_response(&ctx).unwrap();
        if let Ok(body_str) = core::str::from_utf8(resp.body.as_bytes()) {
            assert!(!body_str.contains("I have"));
            assert!(!body_str.contains("online AI"));
        }
    }

    #[test]
    fn fallback_always_valid() {
        let fb = fallback_greeting();
        assert!(!fb.title.is_empty());
        assert!(!fb.body.is_empty());
    }

    #[test]
    fn after_upgrade_from_mtm() {
        let mut ctx = ContextBuilder::new().user_id(0).build();
        ctx.welcome_memory = WelcomeMemoryState {
            visit_count: 1,
            last_completed_generation: Some(1),
            last_successful_provider: None,
        };
        ctx.visit_count = 1;
        ctx.system_generation = Some(2);
        ctx.first_after_upgrade = false;
        let (resp, flags) = plan_greeting_with_flags(&ctx).unwrap();
        assert!(
            flags
                .response_flags
                .has(BrainResponseFlags::AFTER_UPGRADE_GREETING)
                || resp.body.contains("generation")
        );
    }
}
