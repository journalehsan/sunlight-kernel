use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceType {
    MainDocument,
    Stylesheet,
    Script,
    Image,
    Frame,
    Media,
    Preload,
    Prefetch,
    Navigation,
    Other,
}

impl ResourceType {
    pub const fn label(self) -> &'static str {
        match self {
            Self::MainDocument => "Document",
            Self::Stylesheet => "Stylesheet",
            Self::Script => "Script",
            Self::Image => "Image",
            Self::Frame => "Frame",
            Self::Media => "Media",
            Self::Preload => "Preload",
            Self::Prefetch => "Prefetch",
            Self::Navigation => "Navigation",
            Self::Other => "Other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourcePriority {
    MainDocument,
    RenderCritical,
    Embedded,
    ExplicitPreload,
    ExplicitPrefetch,
    Navigation,
}

impl ResourcePriority {
    pub const fn label(self) -> &'static str {
        match self {
            Self::MainDocument => "Main document",
            Self::RenderCritical => "Render-critical",
            Self::Embedded => "Embedded",
            Self::ExplicitPreload => "Preload",
            Self::ExplicitPrefetch => "Prefetch",
            Self::Navigation => "Navigation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestState {
    Queued,
    Connecting,
    Receiving,
    Complete,
    Failed,
    Prefetched,
}

impl RequestState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Connecting => "Connecting",
            Self::Receiving => "Receiving",
            Self::Complete => "Complete",
            Self::Failed => "Failed",
            Self::Prefetched => "Prefetched",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkRequestEntry {
    pub request_id: u64,
    pub sequence_number: usize,
    pub method: String,
    pub original_url: String,
    pub final_url: Option<String>,
    pub resource_type: ResourceType,
    pub priority: ResourcePriority,
    pub status_code: Option<u16>,
    pub status_text: Option<String>,
    pub duration_ms: Option<u64>,
    pub response_body_size: Option<usize>,
    pub content_type: Option<String>,
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
    pub response_body: Option<Vec<u8>>,
    pub request_state: RequestState,
    pub from_cache: Option<bool>,
    pub from_prefetch: Option<bool>,
    pub error_text: Option<String>,
}

impl NetworkRequestEntry {
    pub fn request_url(&self) -> &str {
        self.original_url.as_str()
    }

    pub fn final_url_or_requested(&self) -> &str {
        self.final_url
            .as_deref()
            .unwrap_or(self.original_url.as_str())
    }

    pub fn display_name(&self) -> &str {
        let without_query = self
            .final_url_or_requested()
            .split_once('?')
            .map_or(self.final_url_or_requested(), |(path, _)| path);
        let without_fragment = without_query
            .split_once('#')
            .map_or(without_query, |(path, _)| path);
        let segment = without_fragment.rsplit('/').next().unwrap_or_default();
        if segment.is_empty() {
            self.final_url_or_requested()
        } else {
            segment
        }
    }

    pub fn status_display(&self) -> String {
        match (self.status_code, self.status_text.as_deref()) {
            (Some(code), Some(text)) if !text.is_empty() => format!("{code} {text}"),
            (Some(code), _) => code.to_string(),
            (None, _) => self.request_state.label().to_string(),
        }
    }

    pub fn response_body_size_display(&self) -> String {
        format_body_size(self.response_body_size)
    }

    pub fn duration_display(&self) -> String {
        format_duration_ms(self.duration_ms)
    }
}

pub fn format_duration_ms(duration_ms: Option<u64>) -> String {
    duration_ms.map_or_else(|| String::from("n/a"), |ms| format!("{ms} ms"))
}

pub fn format_body_size(body_size: Option<usize>) -> String {
    body_size.map_or_else(|| String::from("n/a"), |size| format!("{size} bytes"))
}

pub fn format_header_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut uppercase_next = true;
    for ch in name.chars() {
        if ch == '-' {
            out.push(ch);
            uppercase_next = true;
            continue;
        }
        if uppercase_next {
            out.push(ch.to_ascii_uppercase());
            uppercase_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct PageNetworkSession {
    entries: Vec<NetworkRequestEntry>,
    next_request_id: u64,
}

impl Default for PageNetworkSession {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            next_request_id: 1,
        }
    }
}

impl PageNetworkSession {
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn entries(&self) -> &[NetworkRequestEntry] {
        &self.entries
    }

    pub fn entries_mut(&mut self) -> &mut [NetworkRequestEntry] {
        &mut self.entries
    }

    pub fn request_id_at(&self, index: usize) -> Option<u64> {
        self.entries.get(index).map(|entry| entry.request_id)
    }

    pub fn index_of(&self, request_id: u64) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.request_id == request_id)
    }

    pub fn entry(&self, request_id: u64) -> Option<&NetworkRequestEntry> {
        self.entries
            .iter()
            .find(|entry| entry.request_id == request_id)
    }

    pub fn entry_mut(&mut self, request_id: u64) -> Option<&mut NetworkRequestEntry> {
        self.entries
            .iter_mut()
            .find(|entry| entry.request_id == request_id)
    }

    pub fn begin_request(
        &mut self,
        method: &str,
        original_url: String,
        request_headers: Vec<(String, String)>,
        resource_type: ResourceType,
        priority: ResourcePriority,
        request_state: RequestState,
    ) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        let index = self.entries.len();
        self.entries.push(NetworkRequestEntry {
            request_id,
            sequence_number: index + 1,
            method: String::from(method),
            original_url,
            final_url: None,
            resource_type,
            priority,
            status_code: None,
            status_text: None,
            duration_ms: None,
            response_body_size: None,
            content_type: None,
            request_headers,
            response_headers: Vec::new(),
            response_body: None,
            request_state,
            from_cache: None,
            from_prefetch: None,
            error_text: None,
        });
        request_id
    }

    pub fn begin_main_document(
        &mut self,
        method: &str,
        original_url: String,
        request_headers: Vec<(String, String)>,
    ) -> u64 {
        self.begin_request(
            method,
            original_url,
            request_headers,
            ResourceType::MainDocument,
            ResourcePriority::MainDocument,
            RequestState::Queued,
        )
    }

    pub fn set_request_state(&mut self, request_id: u64, request_state: RequestState) {
        if let Some(entry) = self.entry_mut(request_id) {
            entry.request_state = request_state;
        }
    }

    pub fn complete_request(
        &mut self,
        request_id: u64,
        final_url: Option<String>,
        status_code: u16,
        status_text: String,
        duration_ms: Option<u64>,
        response_body_size: Option<usize>,
        content_type: Option<String>,
        response_headers: Vec<(String, String)>,
        response_body: Option<Vec<u8>>,
        from_cache: Option<bool>,
        from_prefetch: Option<bool>,
    ) {
        if let Some(entry) = self.entry_mut(request_id) {
            entry.final_url = final_url;
            entry.status_code = Some(status_code);
            entry.status_text = Some(status_text);
            entry.duration_ms = duration_ms;
            entry.response_body_size = response_body_size;
            entry.content_type = content_type;
            entry.response_headers = response_headers;
            entry.response_body = response_body;
            entry.from_cache = from_cache;
            entry.from_prefetch = from_prefetch;
            entry.error_text = None;
            entry.request_state = if from_prefetch == Some(true) {
                RequestState::Prefetched
            } else {
                RequestState::Complete
            };
        }
    }

    pub fn fail_request(
        &mut self,
        request_id: u64,
        final_url: Option<String>,
        status_code: Option<u16>,
        status_text: Option<String>,
        error_text: impl Into<String>,
    ) {
        if let Some(entry) = self.entry_mut(request_id) {
            if final_url.is_some() {
                entry.final_url = final_url;
            }
            entry.status_code = status_code;
            entry.status_text = status_text;
            entry.request_state = RequestState::Failed;
            entry.error_text = Some(error_text.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_document_entry_starts_queued() {
        let mut session = PageNetworkSession::default();
        let request_id =
            session.begin_main_document("GET", String::from("https://example.com/"), Vec::new());
        let entry = session.entry(request_id).unwrap();
        assert_eq!(entry.request_id, request_id);
        assert_eq!(entry.sequence_number, 1);
        assert_eq!(entry.method, "GET");
        assert_eq!(entry.resource_type, ResourceType::MainDocument);
        assert_eq!(entry.priority, ResourcePriority::MainDocument);
        assert_eq!(entry.request_state, RequestState::Queued);
    }

    #[test]
    fn completing_request_attaches_response_fields() {
        let mut session = PageNetworkSession::default();
        let request_id =
            session.begin_main_document("GET", String::from("https://example.com/"), Vec::new());
        session.set_request_state(request_id, RequestState::Connecting);
        session.complete_request(
            request_id,
            Some(String::from("https://example.com/final")),
            200,
            String::from("OK"),
            Some(42),
            Some(512),
            Some(String::from("text/html")),
            vec![(String::from("content-type"), String::from("text/html"))],
            None,
            Some(false),
            Some(false),
        );

        let entry = session.entry(request_id).unwrap();
        assert_eq!(entry.request_state, RequestState::Complete);
        assert_eq!(entry.status_code, Some(200));
        assert_eq!(entry.status_text.as_deref(), Some("OK"));
        assert_eq!(
            entry.final_url.as_deref(),
            Some("https://example.com/final")
        );
        assert_eq!(entry.duration_ms, Some(42));
        assert_eq!(entry.response_body_size, Some(512));
        assert_eq!(entry.content_type.as_deref(), Some("text/html"));
        assert_eq!(entry.response_headers.len(), 1);
    }

    #[test]
    fn failed_request_preserves_entry_and_error() {
        let mut session = PageNetworkSession::default();
        let request_id =
            session.begin_main_document("GET", String::from("https://example.com/"), Vec::new());
        session.set_request_state(request_id, RequestState::Connecting);
        session.fail_request(request_id, None, None, None, "connection timed out");

        let entry = session.entry(request_id).unwrap();
        assert_eq!(entry.request_state, RequestState::Failed);
        assert_eq!(entry.error_text.as_deref(), Some("connection timed out"));
        assert_eq!(entry.status_code, None);
    }

    #[test]
    fn clearing_session_resets_entries_for_new_page() {
        let mut session = PageNetworkSession::default();
        session.begin_main_document("GET", String::from("https://example.com/"), Vec::new());
        session.clear();
        let request_id =
            session.begin_main_document("GET", String::from("https://example.org/"), Vec::new());
        assert_eq!(session.entry(request_id).unwrap().sequence_number, 1);
        assert_eq!(session.entries().len(), 1);
    }

    #[test]
    fn request_ids_remain_stable_across_entries() {
        let mut session = PageNetworkSession::default();
        let first =
            session.begin_main_document("GET", String::from("https://example.com/"), Vec::new());
        let second =
            session.begin_main_document("GET", String::from("https://example.org/"), Vec::new());
        assert_ne!(first, second);
        assert_eq!(session.request_id_at(0), Some(first));
        assert_eq!(session.request_id_at(1), Some(second));
        assert_eq!(session.index_of(second), Some(1));
    }
}
