use crate::context::{BrainContext, ContextBuilder, GroundedContextBuilder};
use crate::diagnostics::BrainDiagnostics;
use crate::error::{BrainError, BrainResult};
use crate::greeting;
use crate::grounded::{AuthIdentity, BrainContextSource, FactKind, GroundedFact};
use crate::mtm::{BrainPreferences, GreetingStyle, WelcomeMemoryState};
use crate::protocol::{BrainRequestWire, BrainResponseWire};
use crate::provenance::{BrainProviderKind, BrainResponseFlags, BrainResponseMeta};

pub struct CognitivePipeline {
    pub diagnostics: BrainDiagnostics,
}

impl CognitivePipeline {
    pub fn new() -> Self {
        Self {
            diagnostics: BrainDiagnostics::new(),
        }
    }

    pub fn handle_request(&mut self, request: &BrainRequestWire) -> BrainResponseWire {
        self.diagnostics.inc_requests();

        if let Err(_e) = self.validate_request(request) {
            self.diagnostics.inc_rejected();
            self.diagnostics.set_error(1);
            return BrainResponseWire::error(1, request.request_id);
        }

        let request_kind = match BrainRequestKindWire::from_u16(request.request_kind) {
            Some(k) => k,
            None => {
                self.diagnostics.inc_rejected();
                self.diagnostics.set_error(2);
                return BrainResponseWire::error(2, request.request_id);
            }
        };

        match request_kind {
            BrainRequestKindWire::Greeting => {
                self.diagnostics.inc_greeting();
                self.handle_greeting(request)
            }
            BrainRequestKindWire::Summary => {
                self.diagnostics.inc_rejected();
                BrainResponseWire::error(3, request.request_id)
            }
            BrainRequestKindWire::Suggestion => {
                self.diagnostics.inc_rejected();
                BrainResponseWire::error(3, request.request_id)
            }
        }
    }

    pub fn handle_request_grounded(
        &mut self,
        request: &BrainRequestWire,
        identity: &AuthIdentity,
        sources: &[&dyn BrainContextSource],
    ) -> (BrainResponseWire, BrainResponseMeta) {
        self.diagnostics.inc_requests();

        if let Err(_e) = self.validate_request(request) {
            self.diagnostics.inc_rejected();
            self.diagnostics.set_error(1);
            return (
                BrainResponseWire::error(1, request.request_id),
                BrainResponseMeta::empty(),
            );
        }

        let request_kind = match BrainRequestKindWire::from_u16(request.request_kind) {
            Some(k) => k,
            None => {
                self.diagnostics.inc_rejected();
                self.diagnostics.set_error(2);
                return (
                    BrainResponseWire::error(2, request.request_id),
                    BrainResponseMeta::empty(),
                );
            }
        };

        match request_kind {
            BrainRequestKindWire::Greeting => {
                self.diagnostics.inc_greeting();
                self.handle_greeting_grounded(request, identity, sources)
            }
            _ => {
                self.diagnostics.inc_rejected();
                (
                    BrainResponseWire::error(3, request.request_id),
                    BrainResponseMeta::empty(),
                )
            }
        }
    }

    fn validate_request(&self, request: &BrainRequestWire) -> BrainResult<()> {
        // Root (uid=0) is a valid local user on SunlightOS (welcome and many
        // services run as uid=0). Do not treat zero ids as unauthorized.
        // Identity is still constrained by the native daemon's PID badge check.

        if request.request_kind == 1 && request.greeting.is_none() {
            return Err(BrainError::InvalidRequest("greeting request missing payload"));
        }

        if request.user_id != 0
            && request.caller_uid != 0
            && request.user_id != request.caller_uid
        {
            return Err(BrainError::Unauthorized);
        }

        Ok(())
    }

    fn handle_greeting(&mut self, request: &BrainRequestWire) -> BrainResponseWire {
        let ctx = match self.build_context(request) {
            Ok(c) => c,
            Err(_e) => {
                self.diagnostics.inc_context_fail();
                self.diagnostics.set_error(10);
                return BrainResponseWire::error(10, request.request_id);
            }
        };

        let greeting_resp = match greeting::plan_greeting_response(&ctx) {
            Ok(r) => r,
            Err(_e) => {
                self.diagnostics.inc_alignment_fail();
                self.diagnostics.set_error(11);
                return BrainResponseWire::error(11, request.request_id);
            }
        };

        self.diagnostics.inc_local();
        self.diagnostics.inc_success();

        BrainResponseWire::greeting(greeting_resp, request.request_id)
    }

    fn handle_greeting_grounded(
        &mut self,
        request: &BrainRequestWire,
        identity: &AuthIdentity,
        sources: &[&dyn BrainContextSource],
    ) -> (BrainResponseWire, BrainResponseMeta) {
        let mut builder = GroundedContextBuilder::new(*identity);
        let mut all_facts: heapless::Vec<GroundedFact, 48> = heapless::Vec::new();

        for source in sources {
            let facts = builder.gather_from(*source);
            for f in facts.iter() {
                let _ = all_facts.push(f.clone());
            }
        }

        let mut ctx = match self.build_context_from_facts(request, &all_facts) {
            Ok(c) => c,
            Err(_) => {
                self.diagnostics.inc_context_fail();
                self.diagnostics.set_error(10);
                return (
                    BrainResponseWire::error(10, request.request_id),
                    BrainResponseMeta::empty(),
                );
            }
        };

        // Apply MTM facts from the fact set into context preferences/state.
        self.apply_mtm_facts(&mut ctx, &all_facts);

        let (greeting_resp, plan_flags) = match greeting::plan_greeting_with_flags(&ctx) {
            Ok(r) => r,
            Err(_) => {
                self.diagnostics.inc_alignment_fail();
                self.diagnostics.set_error(11);
                return (
                    BrainResponseWire::error(11, request.request_id),
                    BrainResponseMeta::empty(),
                );
            }
        };
        let greeting_resp = match greeting::align_and_shape(greeting_resp) {
            Ok(r) => r,
            Err(_) => {
                self.diagnostics.inc_alignment_fail();
                self.diagnostics.set_error(11);
                return (
                    BrainResponseWire::error(11, request.request_id),
                    BrainResponseMeta::empty(),
                );
            }
        };

        self.diagnostics.inc_local();
        self.diagnostics.inc_success();
        if plan_flags
            .response_flags
            .has(BrainResponseFlags::FIRST_VISIT_GREETING)
        {
            self.diagnostics.inc_first_visit();
        }
        if plan_flags
            .response_flags
            .has(BrainResponseFlags::RETURNING_USER_GREETING)
        {
            self.diagnostics.inc_returning_visit();
        }
        if plan_flags
            .response_flags
            .has(BrainResponseFlags::AFTER_UPGRADE_GREETING)
        {
            self.diagnostics.inc_after_upgrade();
        }
        if plan_flags.machine_summary_included {
            self.diagnostics.inc_machine_summary();
        }
        if plan_flags.index_status_included {
            self.diagnostics.inc_index_status();
        }

        let used_persisted = all_facts.iter().any(|f| {
            matches!(
                f.kind,
                FactKind::VisitCount
                    | FactKind::GreetingStyle
                    | FactKind::ShowMachineSummary
                    | FactKind::ShowIndexStatus
                    | FactKind::LastCompletedGeneration
            )
        });

        let mut response_flags = plan_flags.response_flags;
        if builder.sources_degraded().0 != 0 {
            response_flags.set(BrainResponseFlags::DEGRADED_CONTEXT);
        }
        if used_persisted {
            response_flags.set(BrainResponseFlags::USED_PERSISTED_MEMORY);
        }

        let meta = BrainResponseMeta {
            provider: BrainProviderKind::LocalBounded,
            sources_consulted: builder.sources_consulted(),
            sources_succeeded: builder.sources_succeeded(),
            sources_degraded: builder.sources_degraded(),
            fact_count: all_facts.len() as u8,
            generation_time_us: 0,
            used_persisted_context: used_persisted,
            response_flags,
        };

        (
            BrainResponseWire::greeting(greeting_resp, request.request_id),
            meta,
        )
    }

    fn apply_mtm_facts(&self, ctx: &mut BrainContext, facts: &[GroundedFact]) {
        let mut welcome = WelcomeMemoryState::default();
        let mut prefs = BrainPreferences::default();
        for fact in facts {
            match fact.kind {
                FactKind::VisitCount => {
                    if let Ok(v) = fact.value.parse::<u32>() {
                        welcome.visit_count = v;
                        ctx.visit_count = v;
                    }
                }
                FactKind::LastCompletedGeneration => {
                    if let Ok(v) = fact.value.parse::<u64>() {
                        welcome.last_completed_generation = Some(v);
                    }
                }
                FactKind::GreetingStyle => {
                    if let Some(s) = GreetingStyle::from_str(fact.value.as_str()) {
                        prefs.greeting_style = s;
                    }
                }
                FactKind::ShowMachineSummary => {
                    prefs.show_machine_summary = fact.value.as_str() != "0";
                }
                FactKind::ShowIndexStatus => {
                    prefs.show_index_status = fact.value.as_str() == "1";
                }
                FactKind::IndexReady => {
                    ctx.index_ready = fact.value.as_str() == "1";
                }
                FactKind::IndexedSourceCount => {
                    if let Ok(v) = fact.value.parse::<u64>() {
                        ctx.indexed_source_count = Some(v);
                    }
                }
                FactKind::MemoryDbHealthy => {
                    ctx.memorydb_healthy = fact.value.as_str() == "1";
                }
                FactKind::SystemGeneration | FactKind::MemoryDbGeneration => {
                    if let Ok(v) = fact.value.parse::<u64>() {
                        if ctx.system_generation.is_none() {
                            ctx.system_generation = Some(v);
                        }
                    }
                }
                _ => {}
            }
        }
        ctx.welcome_memory = welcome;
        ctx.preferences = prefs;
    }

    fn build_context_from_facts(
        &self,
        request: &BrainRequestWire,
        facts: &[GroundedFact],
    ) -> BrainResult<BrainContext> {
        let mut builder = ContextBuilder::new().user_id(request.user_id);

        if request.session_id != 0 {
            builder = builder.session_id(Some(request.session_id));
        }

        for fact in facts {
            match fact.kind {
                FactKind::FirstLogin => {
                    builder = builder.first_login(fact.value.as_str() == "1" || !fact.value.is_empty());
                }
                FactKind::FirstAfterUpgrade => {
                    builder =
                        builder.first_after_upgrade(fact.value.as_str() == "1" || !fact.value.is_empty());
                }
                FactKind::RamMib => {
                    if let Ok(v) = fact.value.parse::<u32>() {
                        builder = builder.ram_mib(Some(v));
                    }
                }
                FactKind::CpuCores => {
                    if let Ok(v) = fact.value.parse::<u32>() {
                        builder = builder.cpu_cores(Some(v));
                    }
                }
                FactKind::OsVersion => {
                    builder = builder.sunlight_version(fact.value.as_str());
                }
                _ => {}
            }
        }

        if let Some(ref g) = request.greeting {
            if !g.display_name.is_empty() {
                builder = builder.user_display_name(&g.display_name);
            }
            if !g.sunlight_version.is_empty() {
                builder = builder.sunlight_version(&g.sunlight_version);
            }
            if g.cpu_cores > 0 {
                builder = builder.cpu_cores(Some(g.cpu_cores));
            }
            if g.ram_mib > 0 {
                builder = builder.ram_mib(Some(g.ram_mib));
            }
            if !g.device_class.is_empty() {
                builder = builder.device_class(&g.device_class);
            }
            if !g.model_name.is_empty() {
                builder = builder.model_name(&g.model_name);
            }
            if g.screen_w > 0 {
                builder = builder.screen_dims(Some(g.screen_w), Some(g.screen_h));
            }
            builder = builder.first_login(g.first_login != 0);
            builder = builder.first_after_upgrade(g.first_after_upgrade != 0);
        }

        if !request.locale.is_empty() {
            builder = builder.locale(&request.locale);
        }

        let ctx = builder.build();
        ctx.validate()?;
        Ok(ctx)
    }

    fn build_context(&self, request: &BrainRequestWire) -> BrainResult<BrainContext> {
        let mut builder = ContextBuilder::new()
            .user_id(request.user_id);

        if request.session_id != 0 {
            builder = builder.session_id(Some(request.session_id));
        }

        if let Some(ref g) = request.greeting {
            if !g.display_name.is_empty() {
                builder = builder.user_display_name(&g.display_name);
            }
            if !g.sunlight_version.is_empty() {
                builder = builder.sunlight_version(&g.sunlight_version);
            }
            if g.cpu_cores > 0 {
                builder = builder.cpu_cores(Some(g.cpu_cores));
            }
            if g.ram_mib > 0 {
                builder = builder.ram_mib(Some(g.ram_mib));
            }
            if !g.device_class.is_empty() {
                builder = builder.device_class(&g.device_class);
            }
            if !g.model_name.is_empty() {
                builder = builder.model_name(&g.model_name);
            }
            if g.screen_w > 0 {
                builder = builder.screen_dims(Some(g.screen_w), Some(g.screen_h));
            }
            builder = builder.first_login(g.first_login != 0);
            builder = builder.first_after_upgrade(g.first_after_upgrade != 0);
        }

        if !request.locale.is_empty() {
            builder = builder.locale(&request.locale);
        }

        let ctx = builder.build();
        ctx.validate()?;
        Ok(ctx)
    }
}

impl Default for CognitivePipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrainRequestKindWire {
    Greeting,
    Summary,
    Suggestion,
}

impl BrainRequestKindWire {
    pub fn from_u16(v: u16) -> Option<Self> {
        match v {
            1 => Some(Self::Greeting),
            2 => Some(Self::Summary),
            3 => Some(Self::Suggestion),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{GreetingRequestWire, MAX_DEVICE_CLASS_LEN, MAX_MODEL_LEN, MAX_NAME_LEN, MAX_VERSION_LEN};

    fn make_greeting_request(first_login: bool, name: &str) -> BrainRequestWire {
        let mut dn: heapless::String<MAX_NAME_LEN> = heapless::String::new();
        for c in name.chars() {
            let _ = dn.push(c);
        }
        let mut ver: heapless::String<MAX_VERSION_LEN> = heapless::String::new();
        let _ = ver.push_str("0.2.0");
        let mut dc: heapless::String<MAX_DEVICE_CLASS_LEN> = heapless::String::new();
        let _ = dc.push_str("desktop");
        let mut mn: heapless::String<MAX_MODEL_LEN> = heapless::String::new();
        let _ = mn.push_str("TestBox");

        BrainRequestWire {
            request_id: 1,
            caller_uid: 1000,
            user_id: 1000,
            session_id: 42,
            locale_len: 0,
            locale: heapless::String::new(),
            request_kind: 1,
            greeting: Some(GreetingRequestWire {
                welcome_mode: if first_login { 1 } else { 3 },
                first_login: if first_login { 1 } else { 0 },
                first_after_upgrade: 0,
                machine_summary_requested: 1,
                display_name: dn,
                sunlight_version: ver,
                cpu_cores: 8,
                ram_mib: 16384,
                device_class: dc,
                model_name: mn,
                screen_w: 1920,
                screen_h: 1080,
            }),
        }
    }

    #[test]
    fn pipeline_handles_first_login() {
        let mut pipeline = CognitivePipeline::new();
        let req = make_greeting_request(true, "Alice");
        let resp = pipeline.handle_request(&req);

        assert_eq!(resp.response_kind, 1);
        assert_eq!(resp.request_id, 1);
        assert_eq!(resp.provider, 1);
        assert_eq!(resp.confidence, 100);
        assert!(resp.greeting.is_some());
    }

    #[test]
    fn pipeline_handles_return_visit() {
        let mut pipeline = CognitivePipeline::new();
        let req = make_greeting_request(false, "Bob");
        let resp = pipeline.handle_request(&req);

        assert_eq!(resp.response_kind, 1);
        assert!(resp.greeting.is_some());
    }

    #[test]
    fn pipeline_rejects_missing_greeting_payload() {
        let mut pipeline = CognitivePipeline::new();
        let req = BrainRequestWire {
            request_id: 1,
            caller_uid: 0,
            user_id: 0,
            session_id: 0,
            locale_len: 0,
            locale: heapless::String::new(),
            request_kind: 1,
            greeting: None,
        };
        let resp = pipeline.handle_request(&req);
        assert_eq!(resp.response_kind, 0xFFFE);
        assert_eq!(resp.error_code, 1);
        assert!(resp.greeting.is_none());
    }

    #[test]
    fn pipeline_accepts_root_uid_zero() {
        let mut pipeline = CognitivePipeline::new();
        let mut req = make_greeting_request(true, "Root");
        req.caller_uid = 0;
        req.user_id = 0;
        let resp = pipeline.handle_request(&req);
        assert_eq!(resp.response_kind, 1);
        assert!(resp.greeting.is_some());
    }

    #[test]
    fn pipeline_rejects_mismatched_identity() {
        let mut pipeline = CognitivePipeline::new();
        let mut req = make_greeting_request(true, "Alice");
        req.caller_uid = 1000;
        req.user_id = 2000;
        let resp = pipeline.handle_request(&req);
        assert_eq!(resp.response_kind, 0xFFFE);
        assert_eq!(resp.error_code, 1);
    }

    #[test]
    fn pipeline_rejects_unknown_kind() {
        let mut pipeline = CognitivePipeline::new();
        let req = BrainRequestWire {
            request_id: 2,
            caller_uid: 1000,
            user_id: 1000,
            session_id: 0,
            locale_len: 0,
            locale: heapless::String::new(),
            request_kind: 99,
            greeting: None,
        };
        let resp = pipeline.handle_request(&req);
        assert_eq!(resp.response_kind, 0xFFFE);
        assert_eq!(resp.error_code, 2);
    }

    #[test]
    fn pipeline_preserves_request_id() {
        let mut pipeline = CognitivePipeline::new();
        let req = make_greeting_request(true, "Alice");
        let req2 = BrainRequestWire { request_id: 999, ..req.clone() };
        let resp2 = pipeline.handle_request(&req2);
        assert_eq!(resp2.request_id, 999);
    }

    #[test]
    fn diagnostics_updated() {
        let mut pipeline = CognitivePipeline::new();
        let req = make_greeting_request(true, "Alice");
        let _ = pipeline.handle_request(&req);

        assert_eq!(pipeline.diagnostics.requests_total.load(core::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(pipeline.diagnostics.requests_greeting.load(core::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(pipeline.diagnostics.provider_local_used.load(core::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(pipeline.diagnostics.responses_successful.load(core::sync::atomic::Ordering::Relaxed), 1);
    }

    #[test]
    fn empty_context_still_returns_safe_greeting() {
        let mut pipeline = CognitivePipeline::new();
        let req = BrainRequestWire {
            request_id: 1,
            caller_uid: 1000,
            user_id: 1000,
            session_id: 0,
            locale_len: 0,
            locale: heapless::String::new(),
            request_kind: 1,
            greeting: Some(GreetingRequestWire {
                welcome_mode: 3,
                first_login: 0,
                first_after_upgrade: 0,
                machine_summary_requested: 0,
                display_name: heapless::String::new(),
                sunlight_version: heapless::String::new(),
                cpu_cores: 0,
                ram_mib: 0,
                device_class: heapless::String::new(),
                model_name: heapless::String::new(),
                screen_w: 0,
                screen_h: 0,
            }),
        };
        let resp = pipeline.handle_request(&req);
        assert_eq!(resp.response_kind, 1);
        assert!(resp.greeting.is_some());
    }

    #[test]
    fn multiple_requests_increment_counters() {
        let mut pipeline = CognitivePipeline::new();
        for i in 0..5 {
            let req = make_greeting_request(true, "Test");
            let req = BrainRequestWire { request_id: i, ..req };
            let _ = pipeline.handle_request(&req);
        }
        assert_eq!(pipeline.diagnostics.requests_total.load(core::sync::atomic::Ordering::Relaxed), 5);
        assert_eq!(pipeline.diagnostics.requests_greeting.load(core::sync::atomic::Ordering::Relaxed), 5);
    }

    #[test]
    fn grounded_pipeline_returns_meta() {
        use crate::adapters::SessionContextSource;
        let mut pipeline = CognitivePipeline::new();
        let req = make_greeting_request(true, "Alice");
        let identity = AuthIdentity { caller_uid: 1000, caller_pid: 1, session_id: 42 };
        let session_source = SessionContextSource;
        let sources: [&dyn BrainContextSource; 1] = [&session_source];
        let (resp, meta) = pipeline.handle_request_grounded(&req, &identity, &sources);

        assert_eq!(resp.response_kind, 1);
        assert_eq!(meta.provider, BrainProviderKind::LocalBounded);
        assert!(meta.fact_count > 0);
    }
}
