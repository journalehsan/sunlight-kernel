use core::fmt::Write;

use crate::context::BrainContext;
use crate::error::BrainResult;
use crate::protocol::{
    ActionWire, GreetingResponseWire, HighlightWire, MAX_ACTION_LABEL, MAX_GREETING_LEN,
    MAX_HIGHLIGHT_LABEL, MAX_HIGHLIGHT_VALUE,
};

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

pub fn generate_greeting_response(ctx: &BrainContext) -> BrainResult<GreetingResponseWire> {
    let mut title: heapless::String<MAX_GREETING_LEN> = heapless::String::new();
    let mut body: heapless::String<MAX_GREETING_LEN> = heapless::String::new();

    if ctx.first_login {
        push_greeting_str(&mut title, "Welcome to SunlightOS");
        if !ctx.user_display_name.is_empty() {
            push_greeting_str(&mut body, "Hello, ");
            push_greeting_str(&mut body, &ctx.user_display_name);
            push_greeting_str(&mut body, ". ");
        }
        push_greeting_str(&mut body, "Your desktop is ready. Take a short tour to get acquainted.");
    } else if ctx.first_after_upgrade {
        push_greeting_str(&mut title, "Welcome Back");
        if !ctx.user_display_name.is_empty() {
            push_greeting_str(&mut body, "Hello, ");
            push_greeting_str(&mut body, &ctx.user_display_name);
            push_greeting_str(&mut body, ". ");
        }
        push_greeting_str(&mut body, "SunlightOS has been updated with improvements. Everything should feel familiar.");
    } else {
        push_greeting_str(&mut title, "Welcome");
        if !ctx.user_display_name.is_empty() {
            push_greeting_str(&mut body, "Hello, ");
            push_greeting_str(&mut body, &ctx.user_display_name);
            push_greeting_str(&mut body, ". ");
        }
        push_greeting_str(&mut body, "Your desktop is ready. Browse the tour anytime from the Welcome Center.");
    }

    let mut highlights: heapless::Vec<HighlightWire, { crate::protocol::MAX_HIGHLIGHTS }> =
        heapless::Vec::new();

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
            if ram >= 1024 {
                let _ = write!(&mut value, "{} GiB", ram / 1024);
            } else {
                let _ = write!(&mut value, "{} MiB", ram);
            }
            let _ = highlights.push(HighlightWire {
                kind: 2,
                label: s("Memory"),
                value,
            });
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

    if ctx.first_login {
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

    Ok(GreetingResponseWire {
        title,
        body,
        highlights,
        suggested_actions: actions,
    })
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
    let resp = if ctx.first_login {
        generate_first_login_greeting(ctx)?
    } else if ctx.first_after_upgrade {
        generate_upgrade_greeting(ctx)?
    } else {
        generate_return_greeting(ctx)?
    };
    align_and_shape(resp)
}

fn generate_first_login_greeting(ctx: &BrainContext) -> BrainResult<GreetingResponseWire> {
    generate_greeting_response(ctx)
}

fn generate_upgrade_greeting(ctx: &BrainContext) -> BrainResult<GreetingResponseWire> {
    generate_greeting_response(ctx)
}

fn generate_return_greeting(ctx: &BrainContext) -> BrainResult<GreetingResponseWire> {
    generate_greeting_response(ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ContextBuilder;

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
        assert!(resp.body.contains("Alice"));
        assert!(resp.body.contains("desktop is ready"));
        assert!(!resp.suggested_actions.is_empty());
        assert!(!resp.highlights.is_empty());
    }

    #[test]
    fn first_after_upgrade_greeting() {
        let ctx = ContextBuilder::new()
            .user_id(1000)
            .first_after_upgrade(true)
            .sunlight_version("0.3.0")
            .build();
        let resp = plan_greeting_response(&ctx).unwrap();
        assert!(resp.title.contains("Back") || resp.body.contains("updated"));
    }

    #[test]
    fn return_visit_greeting() {
        let ctx = ContextBuilder::new()
            .user_id(1000)
            .sunlight_version("0.2.0")
            .build();
        let resp = plan_greeting_response(&ctx).unwrap();
        assert!(!resp.title.is_empty());
        assert!(!resp.body.is_empty());
    }

    #[test]
    fn empty_context_returns_safe_greeting() {
        let ctx = ContextBuilder::new().user_id(1000).build();
        let resp = plan_greeting_response(&ctx).unwrap();
        assert!(!resp.title.is_empty());
        assert!(!resp.body.is_empty());
    }

    #[test]
    fn highlights_bounded() {
        let ctx = ContextBuilder::new()
            .user_id(1000)
            .cpu_cores(Some(8))
            .ram_mib(Some(16384))
            .model_name("TestBox")
            .sunlight_version("0.2.0")
            .build();
        let resp = generate_greeting_response(&ctx).unwrap();
        assert!(resp.highlights.len() <= crate::protocol::MAX_HIGHLIGHTS);
    }

    #[test]
    fn suggested_actions_bounded_and_safe() {
        let ctx = ContextBuilder::new()
            .user_id(1000)
            .first_login(true)
            .build();
        let resp = generate_greeting_response(&ctx).unwrap();
        assert!(resp.suggested_actions.len() <= crate::protocol::MAX_ACTIONS);
        for a in &resp.suggested_actions {
            assert!(a.kind != 0xFF || a.label.contains("Placeholder"));
            assert!(!a.label.is_empty());
        }
    }

    #[test]
    fn no_overclaiming_text() {
        let ctx = ContextBuilder::new().user_id(1000).build();
        let resp = plan_greeting_response(&ctx).unwrap();
        if let Ok(body_str) = core::str::from_utf8(resp.body.as_bytes()) {
            assert!(!body_str.contains("I have"));
            assert!(!body_str.contains("I performed"));
            assert!(!body_str.contains("I launched"));
            assert!(!body_str.contains("I modified"));
            assert!(!body_str.contains("online AI"));
        }
    }

    #[test]
    fn fallback_always_valid() {
        let fb = fallback_greeting();
        assert!(!fb.title.is_empty());
        assert!(!fb.body.is_empty());
        assert!(fb.highlights.is_empty());
        assert!(fb.suggested_actions.is_empty());
    }

    #[test]
    fn alignment_rejects_empty_title() {
        let mut resp = fallback_greeting();
        resp.title.clear();
        let result = align_and_shape(resp);
        assert!(result.is_err());
    }

    #[test]
    fn alignment_falls_back_on_empty_body() {
        let mut resp = fallback_greeting();
        resp.body.clear();
        let result = align_and_shape(resp).unwrap();
        assert!(!result.title.is_empty());
        assert!(!result.body.is_empty());
    }
}
