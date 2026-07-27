use core::fmt::Write;

use crate::error::BrainResult;
use crate::protocol::{
    MAX_DEVICE_CLASS_LEN, MAX_GREETING_LEN, MAX_LOCALE_LEN, MAX_MODEL_LEN,
    MAX_NAME_LEN, MAX_VERSION_LEN,
};

#[derive(Debug, Clone)]
pub struct BrainContext {
    pub user_display_name: heapless::String<MAX_NAME_LEN>,
    pub user_id: u64,
    pub session_id: Option<u64>,
    pub locale: heapless::String<MAX_LOCALE_LEN>,
    pub sunlight_version: heapless::String<MAX_VERSION_LEN>,
    pub first_login: bool,
    pub first_after_upgrade: bool,
    pub cpu_cores: Option<u32>,
    pub ram_mib: Option<u32>,
    pub device_class: heapless::String<MAX_DEVICE_CLASS_LEN>,
    pub model_name: heapless::String<MAX_MODEL_LEN>,
    pub screen_w: Option<u32>,
    pub screen_h: Option<u32>,
    pub network_online: Option<bool>,
    pub docs_indexed: Option<bool>,
}

impl Default for BrainContext {
    fn default() -> Self {
        let mut dv: heapless::String<MAX_DEVICE_CLASS_LEN> = heapless::String::new();
        let _ = dv.push_str("desktop");
        Self {
            user_display_name: heapless::String::new(),
            user_id: 0,
            session_id: None,
            locale: heapless::String::new(),
            sunlight_version: heapless::String::new(),
            first_login: false,
            first_after_upgrade: false,
            cpu_cores: None,
            ram_mib: None,
            device_class: dv,
            model_name: heapless::String::new(),
            screen_w: None,
            screen_h: None,
            network_online: None,
            docs_indexed: None,
        }
    }
}

impl BrainContext {
    pub fn machine_summary_line(&self, buf: &mut heapless::String<MAX_GREETING_LEN>) {
        let mut parts: heapless::Vec<&str, 4> = heapless::Vec::new();
        if let Some(cores) = self.cpu_cores {
            let mut s: heapless::String<16> = heapless::String::new();
            let _ = core::fmt::write(&mut s, format_args!("{} CPU cores", cores));
            let _ = parts.push("cpu");
        }
        if let Some(ram) = self.ram_mib {
            if ram >= 1024 {
                let _ = write!(buf, "{} GiB RAM", ram / 1024);
            } else {
                let _ = write!(buf, "{} MiB RAM", ram);
            }
        }
        if !self.model_name.is_empty() {
            if !buf.is_empty() {
                let _ = buf.push_str(", ");
            }
            let _ = buf.push_str(&self.model_name);
        }
    }

    pub fn is_empty_context(&self) -> bool {
        self.user_id == 0
            && self.cpu_cores.is_none()
            && self.ram_mib.is_none()
            && self.model_name.is_empty()
    }

    pub fn validate(&self) -> BrainResult<()> {
        if self.user_id == 0 {
            return Err(crate::error::BrainError::InvalidRequest("missing user_id"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ContextBuilder {
    ctx: BrainContext,
}

impl ContextBuilder {
    pub fn new() -> Self {
        Self {
            ctx: BrainContext::default(),
        }
    }

    pub fn user_display_name(mut self, name: &str) -> Self {
        let mut s: heapless::String<MAX_NAME_LEN> = heapless::String::new();
        for c in name.chars().take(MAX_NAME_LEN) {
            let _ = s.push(c);
        }
        self.ctx.user_display_name = s;
        self
    }

    pub fn user_id(mut self, uid: u64) -> Self {
        self.ctx.user_id = uid;
        self
    }

    pub fn session_id(mut self, sid: Option<u64>) -> Self {
        self.ctx.session_id = sid;
        self
    }

    pub fn locale(mut self, locale: &str) -> Self {
        let mut s: heapless::String<MAX_LOCALE_LEN> = heapless::String::new();
        for c in locale.chars().take(MAX_LOCALE_LEN) {
            let _ = s.push(c);
        }
        self.ctx.locale = s;
        self
    }

    pub fn sunlight_version(mut self, version: &str) -> Self {
        let mut s: heapless::String<MAX_VERSION_LEN> = heapless::String::new();
        for c in version.chars().take(MAX_VERSION_LEN) {
            let _ = s.push(c);
        }
        self.ctx.sunlight_version = s;
        self
    }

    pub fn first_login(mut self, v: bool) -> Self {
        self.ctx.first_login = v;
        self
    }

    pub fn first_after_upgrade(mut self, v: bool) -> Self {
        self.ctx.first_after_upgrade = v;
        self
    }

    pub fn cpu_cores(mut self, n: Option<u32>) -> Self {
        self.ctx.cpu_cores = n;
        self
    }

    pub fn ram_mib(mut self, n: Option<u32>) -> Self {
        self.ctx.ram_mib = n;
        self
    }

    pub fn device_class(mut self, s: &str) -> Self {
        let mut dc: heapless::String<MAX_DEVICE_CLASS_LEN> = heapless::String::new();
        for c in s.chars().take(MAX_DEVICE_CLASS_LEN) {
            let _ = dc.push(c);
        }
        self.ctx.device_class = dc;
        self
    }

    pub fn model_name(mut self, s: &str) -> Self {
        let mut mn: heapless::String<MAX_MODEL_LEN> = heapless::String::new();
        for c in s.chars().take(MAX_MODEL_LEN) {
            let _ = mn.push(c);
        }
        self.ctx.model_name = mn;
        self
    }

    pub fn screen_dims(mut self, w: Option<u32>, h: Option<u32>) -> Self {
        self.ctx.screen_w = w;
        self.ctx.screen_h = h;
        self
    }

    pub fn network_online(mut self, v: Option<bool>) -> Self {
        self.ctx.network_online = v;
        self
    }

    pub fn docs_indexed(mut self, v: Option<bool>) -> Self {
        self.ctx.docs_indexed = v;
        self
    }

    pub fn build(self) -> BrainContext {
        self.ctx
    }

    pub fn build_minimal(uid: u64) -> BrainContext {
        Self::new().user_id(uid).build()
    }
}

impl Default for ContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_builder_full() {
        let ctx = ContextBuilder::new()
            .user_display_name("Alice")
            .user_id(1000)
            .session_id(Some(42))
            .locale("en")
            .sunlight_version("0.2.0")
            .first_login(true)
            .cpu_cores(Some(8))
            .ram_mib(Some(16384))
            .device_class("desktop")
            .model_name("TestBox")
            .screen_dims(Some(1920), Some(1080))
            .network_online(Some(true))
            .build();

        assert_eq!(ctx.user_display_name, "Alice");
        assert_eq!(ctx.user_id, 1000);
        assert_eq!(ctx.session_id, Some(42));
        assert_eq!(ctx.locale, "en");
        assert_eq!(ctx.sunlight_version, "0.2.0");
        assert!(ctx.first_login);
        assert_eq!(ctx.cpu_cores, Some(8));
        assert_eq!(ctx.ram_mib, Some(16384));
        assert_eq!(ctx.model_name, "TestBox");
        assert_eq!(ctx.screen_w, Some(1920));
        assert_eq!(ctx.screen_h, Some(1080));
        assert_eq!(ctx.network_online, Some(true));
        assert!(!ctx.is_empty_context());
    }

    #[test]
    fn empty_context_is_detected() {
        let ctx = BrainContext::default();
        assert!(ctx.is_empty_context());
    }

    #[test]
    fn minimal_context_safe() {
        let ctx = ContextBuilder::new()
            .user_id(1000)
            .sunlight_version("0.1.0")
            .build();
        assert!(!ctx.is_empty_context());
        assert_eq!(ctx.user_id, 1000);
        assert_eq!(ctx.sunlight_version, "0.1.0");
        assert!(ctx.model_name.is_empty());
        assert!(ctx.cpu_cores.is_none());
    }

    #[test]
    fn missing_optional_sources_ok() {
        let ctx = ContextBuilder::new()
            .user_id(1000)
            .sunlight_version("0.1.0")
            .build();
        assert!(ctx.network_online.is_none());
        assert!(ctx.docs_indexed.is_none());
        assert!(ctx.model_name.is_empty());
        assert!(ctx.cpu_cores.is_none());
        assert!(ctx.ram_mib.is_none());
    }

    #[test]
    fn machine_summary_text() {
        let ctx = ContextBuilder::new()
            .user_id(1000)
            .cpu_cores(Some(8))
            .ram_mib(Some(16384))
            .model_name("TestMachine")
            .build();
        let mut buf: heapless::String<MAX_GREETING_LEN> = heapless::String::new();
        ctx.machine_summary_line(&mut buf);
        assert!(!buf.is_empty());
        assert!(buf.contains("GiB") || buf.contains("MiB"));
        assert!(buf.contains("TestMachine"));
    }
}
