use crate::error::{BrainError, BrainResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum BrainRequestKind {
    Greeting = 1,
    Summary = 2,
    Suggestion = 3,
}

impl BrainRequestKind {
    pub fn from_u16(v: u16) -> Option<Self> {
        match v {
            1 => Some(Self::Greeting),
            2 => Some(Self::Summary),
            3 => Some(Self::Suggestion),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum BrainResponseKind {
    Greeting = 1,
    Summary = 2,
    Suggestion = 3,
    Error = 0xFFFE,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BrainProviderKind {
    LocalBounded = 1,
    FutureOnline = 2,
    Fallback = 0xFF,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WelcomeMode {
    FirstLogin = 1,
    FirstAfterUpgrade = 2,
    ReturnVisit = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SuggestedActionKind {
    OpenControlPanel = 1,
    OpenFiles = 2,
    OpenTerminal = 3,
    ContinueWelcomeTour = 4,
    Placeholder = 0xFF,
}

impl SuggestedActionKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::OpenControlPanel),
            2 => Some(Self::OpenFiles),
            3 => Some(Self::OpenTerminal),
            4 => Some(Self::ContinueWelcomeTour),
            0xFF => Some(Self::Placeholder),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GreetingHighlightKind {
    MachineCpu = 1,
    MachineRam = 2,
    MachineModel = 3,
    OsVersion = 4,
    SessionCount = 5,
    NetworkOnline = 6,
    DocsIndexed = 7,
}

impl GreetingHighlightKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::MachineCpu),
            2 => Some(Self::MachineRam),
            3 => Some(Self::MachineModel),
            4 => Some(Self::OsVersion),
            5 => Some(Self::SessionCount),
            6 => Some(Self::NetworkOnline),
            7 => Some(Self::DocsIndexed),
            _ => None,
        }
    }
}

// ── bounded constants ──

pub const MAX_GREETING_LEN: usize = 240;
pub const MAX_NAME_LEN: usize = 48;
pub const MAX_LOCALE_LEN: usize = 16;
pub const MAX_VERSION_LEN: usize = 32;
pub const MAX_MODEL_LEN: usize = 48;
pub const MAX_DEVICE_CLASS_LEN: usize = 16;
pub const MAX_HIGHLIGHTS: usize = 8;
pub const MAX_ACTIONS: usize = 4;
pub const MAX_HIGHLIGHT_LABEL: usize = 64;
pub const MAX_HIGHLIGHT_VALUE: usize = 128;
pub const MAX_ACTION_LABEL: usize = 64;

// ── Host-mode (serde) types ──

#[cfg(feature = "host")]
pub mod host_types {
    
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct BrainRequest {
        pub protocol_version: u16,
        pub request_id: u64,
        pub caller_uid: u64,
        pub caller_gid: u64,
        pub user_id: u64,
        pub session_id: Option<u64>,
        pub locale: Option<String>,
        pub request_kind: BrainRequestKindWire,
        pub payload: BrainRequestPayloadWire,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum BrainRequestKindWire {
        Greeting,
        Summary,
        Suggestion,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum BrainRequestPayloadWire {
        Greeting(GreetingRequest),
        Summary(SummaryRequest),
        Suggestion(SuggestionRequest),
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct GreetingRequest {
        pub welcome_mode: WelcomeModeWire,
        pub first_login: bool,
        pub first_after_upgrade: bool,
        pub machine_summary_requested: bool,
        pub display_name: Option<String>,
        pub sunlight_version: String,
        pub cpu_cores: Option<u32>,
        pub ram_mib: Option<u32>,
        pub device_class: Option<String>,
        pub model_name: Option<String>,
        pub screen_w: Option<u32>,
        pub screen_h: Option<u32>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SummaryRequest {
        pub topic: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SuggestionRequest {
        pub context: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum WelcomeModeWire {
        FirstLogin,
        FirstAfterUpgrade,
        ReturnVisit,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct BrainResponse {
        pub protocol_version: u16,
        pub request_id: u64,
        pub response_kind: BrainResponseKindWire,
        pub provider: BrainProviderKindWire,
        pub confidence: u8,
        pub payload: BrainResponsePayloadWire,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum BrainResponseKindWire {
        Greeting,
        Summary,
        Suggestion,
        Error,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum BrainProviderKindWire {
        LocalBounded,
        FutureOnline,
        Fallback,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum BrainResponsePayloadWire {
        Greeting(GreetingResponse),
        Summary(SummaryResponse),
        Suggestion(SuggestionResponse),
        Error(BrainErrorResponse),
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct GreetingResponse {
        pub source: GreetingSourceWire,
        pub title: String,
        pub body: String,
        pub highlights: Vec<GreetingHighlight>,
        pub suggested_actions: Vec<SuggestedAction>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum GreetingSourceWire {
        Local,
        WiseOwl,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct GreetingHighlight {
        pub kind: GreetingHighlightKindWire,
        pub label: String,
        pub value: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum GreetingHighlightKindWire {
        MachineCpu,
        MachineRam,
        MachineModel,
        OsVersion,
        SessionCount,
        NetworkOnline,
        DocsIndexed,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SuggestedAction {
        pub kind: SuggestedActionKindWire,
        pub label: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum SuggestedActionKindWire {
        OpenControlPanel,
        OpenFiles,
        OpenTerminal,
        ContinueWelcomeTour,
        Placeholder,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SummaryResponse {
        pub title: String,
        pub body: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SuggestionResponse {
        pub title: String,
        pub suggestions: Vec<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct BrainErrorResponse {
        pub code: u16,
        pub message: String,
        pub request_id: u64,
    }
}

// ── Native LE wire types ──

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GreetingRequestWire {
    pub welcome_mode: u8,
    pub first_login: u8,
    pub first_after_upgrade: u8,
    pub machine_summary_requested: u8,
    pub display_name: heapless::String<MAX_NAME_LEN>,
    pub sunlight_version: heapless::String<MAX_VERSION_LEN>,
    pub cpu_cores: u32,
    pub ram_mib: u32,
    pub device_class: heapless::String<MAX_DEVICE_CLASS_LEN>,
    pub model_name: heapless::String<MAX_MODEL_LEN>,
    pub screen_w: u32,
    pub screen_h: u32,
}

impl GreetingRequestWire {
    pub fn encode(&self) -> heapless::Vec<u8, 512> {
        let mut out = heapless::Vec::new();
        let _ = out.push(self.welcome_mode);
        let _ = out.push(self.first_login);
        let _ = out.push(self.first_after_upgrade);
        let _ = out.push(self.machine_summary_requested);
        out.extend_from_slice(&self.cpu_cores.to_le_bytes()).ok();
        out.extend_from_slice(&self.ram_mib.to_le_bytes()).ok();
        out.extend_from_slice(&self.screen_w.to_le_bytes()).ok();
        out.extend_from_slice(&self.screen_h.to_le_bytes()).ok();

        let _ = out.push(self.display_name.len() as u8);
        out.extend_from_slice(self.display_name.as_bytes()).ok();

        let _ = out.push(self.sunlight_version.len() as u8);
        out.extend_from_slice(self.sunlight_version.as_bytes()).ok();

        let _ = out.push(self.device_class.len() as u8);
        out.extend_from_slice(self.device_class.as_bytes()).ok();

        let _ = out.push(self.model_name.len() as u8);
        out.extend_from_slice(self.model_name.as_bytes()).ok();

        out
    }

    pub fn decode(data: &[u8]) -> BrainResult<(Self, usize)> {
        if data.len() < 16 {
            return Err(BrainError::TruncatedBody);
        }
        let welcome_mode = data[0];
        let first_login = data[1];
        let first_after_upgrade = data[2];
        let machine_summary_requested = data[3];
        let cpu_cores = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let ram_mib = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let screen_w = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
        let screen_h = u32::from_le_bytes([data[16], data[17], data[18], data[19]]);
        let mut pos: usize = 20;

        let mut display_name: heapless::String<MAX_NAME_LEN> = heapless::String::new();
        if pos < data.len() {
            let len = data[pos] as usize;
            pos += 1;
            let end = (pos + len).min(data.len());
            let slice = &data[pos..end];
            for &b in slice {
                let _ = display_name.push(b as char);
            }
            pos = end;
        }

        let mut sunlight_version: heapless::String<MAX_VERSION_LEN> = heapless::String::new();
        if pos < data.len() {
            let len = data[pos] as usize;
            pos += 1;
            let end = (pos + len).min(data.len());
            let slice = &data[pos..end];
            for &b in slice {
                let _ = sunlight_version.push(b as char);
            }
            pos = end;
        }

        let mut device_class: heapless::String<MAX_DEVICE_CLASS_LEN> = heapless::String::new();
        if pos < data.len() {
            let len = data[pos] as usize;
            pos += 1;
            let end = (pos + len).min(data.len());
            let slice = &data[pos..end];
            for &b in slice {
                let _ = device_class.push(b as char);
            }
            pos = end;
        }

        let mut model_name: heapless::String<MAX_MODEL_LEN> = heapless::String::new();
        if pos < data.len() {
            let len = data[pos] as usize;
            pos += 1;
            let end = (pos + len).min(data.len());
            let slice = &data[pos..end];
            for &b in slice {
                let _ = model_name.push(b as char);
            }
            pos = end;
        }

        Ok((Self {
            welcome_mode,
            first_login,
            first_after_upgrade,
            machine_summary_requested,
            display_name,
            sunlight_version,
            cpu_cores,
            ram_mib,
            device_class,
            model_name,
            screen_w,
            screen_h,
        }, pos))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GreetingResponseWire {
    pub title: heapless::String<MAX_GREETING_LEN>,
    pub body: heapless::String<MAX_GREETING_LEN>,
    pub highlights: heapless::Vec<HighlightWire, MAX_HIGHLIGHTS>,
    pub suggested_actions: heapless::Vec<ActionWire, MAX_ACTIONS>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightWire {
    pub kind: u8,
    pub label: heapless::String<MAX_HIGHLIGHT_LABEL>,
    pub value: heapless::String<MAX_HIGHLIGHT_VALUE>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionWire {
    pub kind: u8,
    pub label: heapless::String<MAX_ACTION_LABEL>,
}

impl GreetingResponseWire {
    pub fn simple(title: &str, body: &str) -> Self {
        let mut t: heapless::String<MAX_GREETING_LEN> = heapless::String::new();
        for c in title.chars() {
            let _ = t.push(c);
        }
        let mut b: heapless::String<MAX_GREETING_LEN> = heapless::String::new();
        for c in body.chars() {
            let _ = b.push(c);
        }
        Self {
            title: t,
            body: b,
            highlights: heapless::Vec::new(),
            suggested_actions: heapless::Vec::new(),
        }
    }

    pub fn encode(&self) -> heapless::Vec<u8, 1024> {
        let mut out = heapless::Vec::new();

        let _ = out.push(self.title.len() as u8);
        out.extend_from_slice(self.title.as_bytes()).ok();

        let _ = out.push(self.body.len() as u8);
        out.extend_from_slice(self.body.as_bytes()).ok();

        let _ = out.push(self.highlights.len() as u8);
        for h in &self.highlights {
            let _ = out.push(h.kind);
            let _ = out.push(h.label.len() as u8);
            out.extend_from_slice(h.label.as_bytes()).ok();
            let _ = out.push(h.value.len() as u8);
            out.extend_from_slice(h.value.as_bytes()).ok();
        }

        let _ = out.push(self.suggested_actions.len() as u8);
        for a in &self.suggested_actions {
            let _ = out.push(a.kind);
            let _ = out.push(a.label.len() as u8);
            out.extend_from_slice(a.label.as_bytes()).ok();
        }

        out
    }

    pub fn decode(data: &[u8]) -> BrainResult<(Self, usize)> {
        if data.is_empty() {
            return Err(BrainError::TruncatedBody);
        }
        let mut pos: usize = 0;

        let title_len = data[pos] as usize;
        pos += 1;
        if pos + title_len > data.len() {
            return Err(BrainError::TruncatedBody);
        }
        let mut title: heapless::String<MAX_GREETING_LEN> = heapless::String::new();
        for &b in &data[pos..pos + title_len] {
            let _ = title.push(b as char);
        }
        pos += title_len;

        let body_len = data[pos] as usize;
        pos += 1;
        if pos + body_len > data.len() {
            return Err(BrainError::TruncatedBody);
        }
        let mut body: heapless::String<MAX_GREETING_LEN> = heapless::String::new();
        for &b in &data[pos..pos + body_len] {
            let _ = body.push(b as char);
        }
        pos += body_len;

        let mut highlights: heapless::Vec<HighlightWire, MAX_HIGHLIGHTS> = heapless::Vec::new();
        let hl_count = data[pos] as usize;
        pos += 1;
        for _ in 0..hl_count {
            if pos >= data.len() {
                return Err(BrainError::TruncatedBody);
            }
            let kind = data[pos];
            pos += 1;

            let label_len = data[pos] as usize;
            pos += 1;
            if pos + label_len > data.len() {
                return Err(BrainError::TruncatedBody);
            }
            let mut label: heapless::String<MAX_HIGHLIGHT_LABEL> = heapless::String::new();
            for &b in &data[pos..pos + label_len] {
                let _ = label.push(b as char);
            }
            pos += label_len;

            let value_len = data[pos] as usize;
            pos += 1;
            if pos + value_len > data.len() {
                return Err(BrainError::TruncatedBody);
            }
            let mut value: heapless::String<MAX_HIGHLIGHT_VALUE> = heapless::String::new();
            for &b in &data[pos..pos + value_len] {
                let _ = value.push(b as char);
            }
            pos += value_len;

            let _ = highlights.push(HighlightWire { kind, label, value });
        }

        let mut suggested_actions: heapless::Vec<ActionWire, MAX_ACTIONS> = heapless::Vec::new();
        if pos < data.len() {
            let ac_count = data[pos] as usize;
            pos += 1;
            for _ in 0..ac_count {
                if pos >= data.len() {
                    return Err(BrainError::TruncatedBody);
                }
                let kind = data[pos];
                pos += 1;

                let label_len = data[pos] as usize;
                pos += 1;
                if pos + label_len > data.len() {
                    return Err(BrainError::TruncatedBody);
                }
                let mut label: heapless::String<MAX_ACTION_LABEL> = heapless::String::new();
                for &b in &data[pos..pos + label_len] {
                    let _ = label.push(b as char);
                }
                pos += label_len;

                let _ = suggested_actions.push(ActionWire { kind, label });
            }
        }

        Ok((Self { title, body, highlights, suggested_actions }, pos))
    }
}

// ── Brain request/response full wire ──

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrainRequestWire {
    pub request_id: u64,
    pub caller_uid: u64,
    pub user_id: u64,
    pub session_id: u64,
    pub locale_len: u8,
    pub locale: heapless::String<MAX_LOCALE_LEN>,
    pub request_kind: u16,
    pub greeting: Option<GreetingRequestWire>,
}

impl BrainRequestWire {
    pub fn encode(&self) -> heapless::Vec<u8, 768> {
        let mut out = heapless::Vec::new();
        out.extend_from_slice(&self.request_id.to_le_bytes()).ok();
        out.extend_from_slice(&self.caller_uid.to_le_bytes()).ok();
        out.extend_from_slice(&self.user_id.to_le_bytes()).ok();
        out.extend_from_slice(&self.session_id.to_le_bytes()).ok();
        let _ = out.push(self.locale_len);
        out.extend_from_slice(self.locale.as_bytes()).ok();
        out.extend_from_slice(&self.request_kind.to_le_bytes()).ok();

        if let Some(ref g) = self.greeting {
            let g_body = g.encode();
            out.extend_from_slice(&(g_body.len() as u16).to_le_bytes()).ok();
            out.extend_from_slice(&g_body).ok();
        } else {
            out.extend_from_slice(&0u16.to_le_bytes()).ok();
        }
        out
    }

    pub fn decode(data: &[u8]) -> BrainResult<(Self, usize)> {
        if data.len() < 38 {
            return Err(BrainError::TruncatedBody);
        }
        let request_id = u64::from_le_bytes(data[0..8].try_into().unwrap());
        let caller_uid = u64::from_le_bytes(data[8..16].try_into().unwrap());
        let user_id = u64::from_le_bytes(data[16..24].try_into().unwrap());
        let session_id = u64::from_le_bytes(data[24..32].try_into().unwrap());
        let locale_len = data[32] as usize;
        if locale_len > MAX_LOCALE_LEN {
            return Err(BrainError::BadEncoding);
        }
        let locale_start = 33;
        let locale_end = locale_start + locale_len;
        if locale_end > data.len() {
            return Err(BrainError::TruncatedBody);
        }
        let mut locale: heapless::String<MAX_LOCALE_LEN> = heapless::String::new();
        for &b in &data[locale_start..locale_end] {
            let _ = locale.push(b as char);
        }
        let pos = locale_end;
        if pos + 2 > data.len() {
            return Err(BrainError::TruncatedBody);
        }
        let request_kind = u16::from_le_bytes([data[pos], data[pos + 1]]);
        let mut pos = pos + 2;

        let greeting = if request_kind == 1 {
            if pos + 2 > data.len() {
                return Err(BrainError::TruncatedBody);
            }
            let g_len = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
            pos += 2;
            if g_len > 0 {
                if pos + g_len > data.len() {
                    return Err(BrainError::TruncatedBody);
                }
                let (g, _consumed) = GreetingRequestWire::decode(&data[pos..pos + g_len])?;
                pos += g_len;
                Some(g)
            } else {
                None
            }
        } else {
            None
        };

        Ok((Self {
            request_id,
            caller_uid,
            user_id,
            session_id,
            locale_len: locale_len as u8,
            locale,
            request_kind,
            greeting,
        }, pos))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrainResponseWire {
    pub request_id: u64,
    pub response_kind: u16,
    pub provider: u8,
    pub confidence: u8,
    pub error_code: u16,
    pub greeting: Option<GreetingResponseWire>,
}

impl BrainResponseWire {
    pub fn greeting(g: GreetingResponseWire, request_id: u64) -> Self {
        Self {
            request_id,
            response_kind: 1,
            provider: 1,
            confidence: 100,
            error_code: 0,
            greeting: Some(g),
        }
    }

    pub fn error(code: u16, request_id: u64) -> Self {
        Self {
            request_id,
            response_kind: 0xFFFE,
            provider: 0xFF,
            confidence: 0,
            error_code: code,
            greeting: None,
        }
    }

    pub fn encode(&self) -> heapless::Vec<u8, 2048> {
        let mut out = heapless::Vec::new();
        out.extend_from_slice(&self.request_id.to_le_bytes()).ok();
        out.extend_from_slice(&self.response_kind.to_le_bytes()).ok();
        let _ = out.push(self.provider);
        let _ = out.push(self.confidence);
        out.extend_from_slice(&self.error_code.to_le_bytes()).ok();

        if let Some(ref g) = self.greeting {
            let g_body = g.encode();
            out.extend_from_slice(&(g_body.len() as u16).to_le_bytes()).ok();
            out.extend_from_slice(&g_body).ok();
        } else {
            out.extend_from_slice(&0u16.to_le_bytes()).ok();
        }
        out
    }

    pub fn decode(data: &[u8]) -> BrainResult<(Self, usize)> {
        if data.len() < 16 {
            return Err(BrainError::TruncatedBody);
        }
        let request_id = u64::from_le_bytes(data[0..8].try_into().unwrap());
        let response_kind = u16::from_le_bytes([data[8], data[9]]);
        let provider = data[10];
        let confidence = data[11];
        let error_code = u16::from_le_bytes([data[12], data[13]]);
        let mut pos: usize = 14;

        let greeting = if response_kind == 1 {
            if pos + 2 > data.len() {
                return Err(BrainError::TruncatedBody);
            }
            let g_len = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
            pos += 2;
            if g_len > 0 {
                if pos + g_len > data.len() {
                    return Err(BrainError::TruncatedBody);
                }
                let (g, _consumed) = GreetingResponseWire::decode(&data[pos..pos + g_len])?;
                pos += g_len;
                Some(g)
            } else {
                None
            }
        } else {
            None
        };

        Ok((Self {
            request_id,
            response_kind,
            provider,
            confidence,
            error_code,
            greeting,
        }, pos))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greeting_request_encode_decode_roundtrip() {
        let mut dn: heapless::String<MAX_NAME_LEN> = heapless::String::new();
        let _ = dn.push_str("Alice");
        let mut v: heapless::String<MAX_VERSION_LEN> = heapless::String::new();
        let _ = v.push_str("0.1.0");
        let mut dc: heapless::String<MAX_DEVICE_CLASS_LEN> = heapless::String::new();
        let _ = dc.push_str("desktop");
        let mut mn: heapless::String<MAX_MODEL_LEN> = heapless::String::new();
        let _ = mn.push_str("TestBox");

        let req = GreetingRequestWire {
            welcome_mode: 1,
            first_login: 1,
            first_after_upgrade: 0,
            machine_summary_requested: 1,
            display_name: dn,
            sunlight_version: v,
            cpu_cores: 8,
            ram_mib: 16384,
            device_class: dc,
            model_name: mn,
            screen_w: 1920,
            screen_h: 1080,
        };
        let encoded = req.encode();
        let (decoded, _c) = GreetingRequestWire::decode(&encoded).unwrap();
        assert_eq!(decoded.welcome_mode, 1);
        assert_eq!(decoded.cpu_cores, 8);
        assert_eq!(decoded.ram_mib, 16384);
        assert_eq!(decoded.display_name, "Alice");
        assert_eq!(decoded.sunlight_version, "0.1.0");
        assert_eq!(decoded.device_class, "desktop");
        assert_eq!(decoded.model_name, "TestBox");
        assert_eq!(decoded.screen_w, 1920);
        assert_eq!(decoded.screen_h, 1080);
    }

    #[test]
    fn greeting_response_encode_decode_roundtrip() {
        let resp = GreetingResponseWire {
            title: heapless::String::try_from("Welcome").unwrap(),
            body: heapless::String::try_from("Your desktop is ready.").unwrap(),
            highlights: heapless::Vec::new(),
            suggested_actions: heapless::Vec::new(),
        };
        let encoded = resp.encode();
        let (decoded, _c) = GreetingResponseWire::decode(&encoded).unwrap();
        assert_eq!(decoded.title, "Welcome");
        assert_eq!(decoded.body, "Your desktop is ready.");
    }

    #[test]
    fn full_request_response_roundtrip() {
        let mut dn: heapless::String<MAX_NAME_LEN> = heapless::String::new();
        let _ = dn.push_str("Test");
        let mut ver: heapless::String<MAX_VERSION_LEN> = heapless::String::new();
        let _ = ver.push_str("1.0");
        let mut dc: heapless::String<MAX_DEVICE_CLASS_LEN> = heapless::String::new();
        let _ = dc.push_str("desktop");
        let mut mn: heapless::String<MAX_MODEL_LEN> = heapless::String::new();
        let _ = mn.push_str("machine");

        let req = BrainRequestWire {
            request_id: 1,
            caller_uid: 1000,
            user_id: 1000,
            session_id: 42,
            locale_len: 0,
            locale: heapless::String::new(),
            request_kind: 1,
            greeting: Some(GreetingRequestWire {
                welcome_mode: 3,
                first_login: 0,
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
        let enc = req.encode();
        let (dec, _c) = BrainRequestWire::decode(&enc).unwrap();
        assert_eq!(dec.request_id, 1);
        assert_eq!(dec.caller_uid, 1000);
        assert_eq!(dec.request_kind, 1);
        assert!(dec.greeting.is_some());
        let g = dec.greeting.unwrap();
        assert_eq!(g.cpu_cores, 4);
        assert_eq!(g.ram_mib, 8192);
    }

    #[test]
    fn bounded_string_limits() {
        let long = "x".repeat(MAX_GREETING_LEN + 10);
        let mut s: heapless::String<MAX_GREETING_LEN> = heapless::String::new();
        for c in long.chars().take(MAX_GREETING_LEN) {
            let _ = s.push(c);
        }
        assert_eq!(s.len(), MAX_GREETING_LEN);
    }

    #[test]
    fn empty_context_returns_safe_greeting() {
        let resp = GreetingResponseWire::simple("Welcome", "Your desktop is ready.");
        let enc = resp.encode();
        let (dec, _) = GreetingResponseWire::decode(&enc).unwrap();
        assert!(!dec.title.is_empty());
        assert!(!dec.body.is_empty());
    }

    #[test]
    fn highlights_bounded() {
        let mut highlights: heapless::Vec<HighlightWire, MAX_HIGHLIGHTS> = heapless::Vec::new();
        for i in 0..MAX_HIGHLIGHTS + 2 {
            let mut label: heapless::String<MAX_HIGHLIGHT_LABEL> = heapless::String::new();
            let _ = label.push_str("test");
            let mut value: heapless::String<MAX_HIGHLIGHT_VALUE> = heapless::String::new();
            let _ = value.push_str("value");
            if i < MAX_HIGHLIGHTS {
                let _ = highlights.push(HighlightWire { kind: i as u8, label, value });
            }
        }
        assert_eq!(highlights.len(), MAX_HIGHLIGHTS);
    }

    #[test]
    fn suggested_actions_bounded() {
        let mut actions: heapless::Vec<ActionWire, MAX_ACTIONS> = heapless::Vec::new();
        for i in 0..MAX_ACTIONS + 2 {
            let mut label: heapless::String<MAX_ACTION_LABEL> = heapless::String::new();
            let _ = label.push_str("action");
            if i < MAX_ACTIONS {
                let _ = actions.push(ActionWire { kind: i as u8, label });
            }
        }
        assert_eq!(actions.len(), MAX_ACTIONS);
    }
}
