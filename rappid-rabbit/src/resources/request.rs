use alloc::{string::String, vec::Vec};

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
    pub sequence_number: usize,
    pub method: String,
    pub resource_url: String,
    pub resource_type: ResourceType,
    pub priority: ResourcePriority,
    pub status: Option<u16>,
    pub duration_ms: Option<u64>,
    pub body_size: Option<usize>,
    pub content_type: Option<String>,
    pub request_state: RequestState,
    pub from_cache: Option<bool>,
    pub from_prefetch: Option<bool>,
    pub error_text: Option<String>,
}

impl NetworkRequestEntry {
    pub fn display_name(&self) -> &str {
        let without_query = self
            .resource_url
            .split_once('?')
            .map_or(self.resource_url.as_str(), |(path, _)| path);
        let without_fragment = without_query
            .split_once('#')
            .map_or(without_query, |(path, _)| path);
        let segment = without_fragment.rsplit('/').next().unwrap_or_default();
        if segment.is_empty() {
            self.resource_url.as_str()
        } else {
            segment
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PageNetworkSession {
    entries: Vec<NetworkRequestEntry>,
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

    pub fn begin_request(
        &mut self,
        method: &str,
        resource_url: String,
        resource_type: ResourceType,
        priority: ResourcePriority,
        request_state: RequestState,
    ) -> usize {
        let index = self.entries.len();
        self.entries.push(NetworkRequestEntry {
            sequence_number: index + 1,
            method: String::from(method),
            resource_url,
            resource_type,
            priority,
            status: None,
            duration_ms: None,
            body_size: None,
            content_type: None,
            request_state,
            from_cache: None,
            from_prefetch: None,
            error_text: None,
        });
        index
    }

    pub fn begin_main_document(&mut self, method: &str, resource_url: String) -> usize {
        self.begin_request(
            method,
            resource_url,
            ResourceType::MainDocument,
            ResourcePriority::MainDocument,
            RequestState::Queued,
        )
    }

    pub fn set_request_state(&mut self, index: usize, request_state: RequestState) {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.request_state = request_state;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete_request(
        &mut self,
        index: usize,
        status: u16,
        duration_ms: Option<u64>,
        body_size: Option<usize>,
        content_type: Option<String>,
        from_cache: Option<bool>,
        from_prefetch: Option<bool>,
    ) {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.status = Some(status);
            entry.duration_ms = duration_ms;
            entry.body_size = body_size;
            entry.content_type = content_type;
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

    pub fn fail_request(&mut self, index: usize, error_text: impl Into<String>) {
        if let Some(entry) = self.entries.get_mut(index) {
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
        let index = session.begin_main_document("GET", String::from("https://example.com/"));
        let entry = &session.entries()[index];
        assert_eq!(entry.sequence_number, 1);
        assert_eq!(entry.method, "GET");
        assert_eq!(entry.resource_type, ResourceType::MainDocument);
        assert_eq!(entry.priority, ResourcePriority::MainDocument);
        assert_eq!(entry.request_state, RequestState::Queued);
    }

    #[test]
    fn completing_request_attaches_response_fields() {
        let mut session = PageNetworkSession::default();
        let index = session.begin_main_document("GET", String::from("https://example.com/"));
        session.set_request_state(index, RequestState::Connecting);
        session.complete_request(
            index,
            200,
            Some(42),
            Some(512),
            Some(String::from("text/html")),
            Some(false),
            Some(false),
        );

        let entry = &session.entries()[index];
        assert_eq!(entry.request_state, RequestState::Complete);
        assert_eq!(entry.status, Some(200));
        assert_eq!(entry.duration_ms, Some(42));
        assert_eq!(entry.body_size, Some(512));
        assert_eq!(entry.content_type.as_deref(), Some("text/html"));
    }

    #[test]
    fn failed_request_preserves_entry_and_error() {
        let mut session = PageNetworkSession::default();
        let index = session.begin_main_document("GET", String::from("https://example.com/"));
        session.set_request_state(index, RequestState::Connecting);
        session.fail_request(index, "connection timed out");

        let entry = &session.entries()[index];
        assert_eq!(entry.request_state, RequestState::Failed);
        assert_eq!(entry.error_text.as_deref(), Some("connection timed out"));
        assert_eq!(entry.status, None);
    }

    #[test]
    fn clearing_session_resets_entries_for_new_page() {
        let mut session = PageNetworkSession::default();
        session.begin_main_document("GET", String::from("https://example.com/"));
        session.clear();
        let index = session.begin_main_document("GET", String::from("https://example.org/"));
        assert_eq!(session.entries()[index].sequence_number, 1);
        assert_eq!(session.entries().len(), 1);
    }
}
