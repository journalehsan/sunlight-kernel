use crate::context::BrainContext;
use crate::error::BrainResult;
use crate::protocol::{BrainRequestWire, BrainResponseWire};

pub trait BrainProvider {
    fn name(&self) -> &'static str;
    fn is_available(&self) -> bool;
    fn handle(
        &self,
        request: &BrainRequestWire,
        ctx: &BrainContext,
    ) -> BrainResult<BrainResponseWire>;
}

pub struct LocalBoundedProvider;

impl BrainProvider for LocalBoundedProvider {
    fn name(&self) -> &'static str {
        "local-bounded"
    }

    fn is_available(&self) -> bool {
        true
    }

    fn handle(
        &self,
        request: &BrainRequestWire,
        ctx: &BrainContext,
    ) -> BrainResult<BrainResponseWire> {
        let greeting = crate::greeting::plan_greeting_response(ctx)?;
        let resp = BrainResponseWire::greeting(greeting, request.request_id);
        Ok(resp)
    }
}

pub struct FutureOnlineProvider;

impl BrainProvider for FutureOnlineProvider {
    fn name(&self) -> &'static str {
        "future-online"
    }

    fn is_available(&self) -> bool {
        false
    }

    fn handle(
        &self,
        _request: &BrainRequestWire,
        _ctx: &BrainContext,
    ) -> BrainResult<BrainResponseWire> {
        Err(crate::error::BrainError::ProviderUnavailable)
    }
}

pub struct ProviderRegistry {
    pub local: LocalBoundedProvider,
    pub future: FutureOnlineProvider,
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self {
            local: LocalBoundedProvider,
            future: FutureOnlineProvider,
        }
    }
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ContextBuilder;

    #[test]
    fn local_provider_always_available() {
        let p = LocalBoundedProvider;
        assert!(p.is_available());
    }

    #[test]
    fn future_provider_not_available() {
        let p = FutureOnlineProvider;
        assert!(!p.is_available());
    }

    #[test]
    fn local_provider_returns_valid_greeting() {
        let ctx = ContextBuilder::new()
            .user_id(1000)
            .user_display_name("Alice")
            .sunlight_version("0.2.0")
            .first_login(true)
            .build();

        let mut dn: heapless::String<{ crate::protocol::MAX_NAME_LEN }> = heapless::String::new();
        let _ = dn.push_str("Alice");
        let mut ver: heapless::String<{ crate::protocol::MAX_VERSION_LEN }> =
            heapless::String::new();
        let _ = ver.push_str("0.2.0");
        let mut dc: heapless::String<{ crate::protocol::MAX_DEVICE_CLASS_LEN }> =
            heapless::String::new();
        let _ = dc.push_str("desktop");
        let mut mn: heapless::String<{ crate::protocol::MAX_MODEL_LEN }> = heapless::String::new();

        let req = BrainRequestWire {
            request_id: 1,
            caller_uid: 1000,
            user_id: 1000,
            session_id: 0,
            locale_len: 0,
            locale: heapless::String::new(),
            request_kind: 1,
            greeting: Some(crate::protocol::GreetingRequestWire {
                welcome_mode: 1,
                first_login: 1,
                first_after_upgrade: 0,
                machine_summary_requested: 1,
                display_name: dn,
                sunlight_version: ver,
                cpu_cores: 4,
                ram_mib: 8192,
                device_class: dc,
                model_name: mn,
                screen_w: 0,
                screen_h: 0,
            }),
        };

        let p = LocalBoundedProvider;
        let resp = p.handle(&req, &ctx).unwrap();
        assert_eq!(resp.response_kind, 1);
        assert_eq!(resp.request_id, 1);
        assert!(resp.greeting.is_some());
    }

    #[test]
    fn provider_timeout_fallback() {
        let p = FutureOnlineProvider;
        assert!(!p.is_available());
        let ctx = ContextBuilder::new().user_id(1000).build();
        let mut dn: heapless::String<{ crate::protocol::MAX_NAME_LEN }> = heapless::String::new();
        let mut ver: heapless::String<{ crate::protocol::MAX_VERSION_LEN }> =
            heapless::String::new();
        let _ = ver.push_str("0.1.0");
        let mut dc: heapless::String<{ crate::protocol::MAX_DEVICE_CLASS_LEN }> =
            heapless::String::new();
        let mut mn: heapless::String<{ crate::protocol::MAX_MODEL_LEN }> = heapless::String::new();
        let req = BrainRequestWire {
            request_id: 1,
            caller_uid: 1000,
            user_id: 1000,
            session_id: 0,
            locale_len: 0,
            locale: heapless::String::new(),
            request_kind: 1,
            greeting: Some(crate::protocol::GreetingRequestWire {
                welcome_mode: 1,
                first_login: 0,
                first_after_upgrade: 0,
                machine_summary_requested: 0,
                display_name: dn,
                sunlight_version: ver,
                cpu_cores: 0,
                ram_mib: 0,
                device_class: dc,
                model_name: mn,
                screen_w: 0,
                screen_h: 0,
            }),
        };
        let result = p.handle(&req, &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn provider_failure_fallback() {
        let p = FutureOnlineProvider;
        let ctx = BrainContext::default();
        let req = BrainRequestWire {
            request_id: 1,
            caller_uid: 1000,
            user_id: 1000,
            session_id: 0,
            locale_len: 0,
            locale: heapless::String::new(),
            request_kind: 1,
            greeting: None,
        };
        let result = p.handle(&req, &ctx);
        assert!(result.is_err());
    }
}
