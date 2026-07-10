use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use crate::resources::request::{
    NetworkRequestEntry, PageNetworkSession, RequestState, ResourcePriority, ResourceType,
};

#[derive(Debug, Clone, Default)]
pub struct NetworkTabState {
    session: PageNetworkSession,
    scroll_offset: usize,
    selected_entry: Option<usize>,
}

impl NetworkTabState {
    pub fn clear_for_new_page(&mut self) {
        self.session.clear();
        self.scroll_offset = 0;
        self.selected_entry = None;
    }

    pub fn begin_main_document_request(&mut self, method: &str, resource_url: String) -> usize {
        self.selected_entry = None;
        self.session.begin_main_document(method, resource_url)
    }

    pub fn begin_request(
        &mut self,
        method: &str,
        resource_url: String,
        resource_type: ResourceType,
        priority: ResourcePriority,
        request_state: RequestState,
    ) -> usize {
        self.session
            .begin_request(method, resource_url, resource_type, priority, request_state)
    }

    pub fn set_request_state(&mut self, index: usize, request_state: RequestState) {
        self.session.set_request_state(index, request_state);
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
        self.session.complete_request(
            index,
            status,
            duration_ms,
            body_size,
            content_type,
            from_cache,
            from_prefetch,
        );
    }

    pub fn fail_request(&mut self, index: usize, error_text: impl Into<String>) {
        self.session.fail_request(index, error_text);
    }

    pub fn entries(&self) -> &[NetworkRequestEntry] {
        self.session.entries()
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn set_scroll_offset(&mut self, scroll_offset: usize) {
        self.scroll_offset = scroll_offset;
    }

    pub fn selected_entry(&self) -> Option<usize> {
        self.selected_entry
    }

    pub fn select_entry(&mut self, selected_entry: Option<usize>) {
        self.selected_entry = selected_entry.filter(|index| *index < self.entries().len());
    }

    pub fn selected_entry_detail_text(&self) -> String {
        let Some(index) = self.selected_entry else {
            return String::from("Select a request to inspect its details.");
        };
        let Some(entry) = self.entries().get(index) else {
            return String::from("Select a request to inspect its details.");
        };

        let mut out = String::new();
        out.push_str("Sequence: ");
        out.push_str(&entry.sequence_number.to_string());
        out.push('\n');
        out.push_str("Method: ");
        out.push_str(&entry.method);
        out.push('\n');
        out.push_str("URL: ");
        out.push_str(&entry.resource_url);
        out.push('\n');
        out.push_str("Type: ");
        out.push_str(entry.resource_type.label());
        out.push('\n');
        out.push_str("Priority: ");
        out.push_str(entry.priority.label());
        out.push('\n');
        out.push_str("State: ");
        out.push_str(entry.request_state.label());
        out.push('\n');
        out.push_str("Status: ");
        out.push_str(
            &entry
                .status
                .map_or_else(|| String::from("n/a"), |status| status.to_string()),
        );
        out.push('\n');
        out.push_str("Duration: ");
        out.push_str(
            &entry
                .duration_ms
                .map_or_else(|| String::from("n/a"), |ms| format!("{ms} ms")),
        );
        out.push('\n');
        out.push_str("Body Size: ");
        out.push_str(
            &entry
                .body_size
                .map_or_else(|| String::from("n/a"), |size| format!("{size} bytes")),
        );
        out.push('\n');
        out.push_str("Content-Type: ");
        out.push_str(entry.content_type.as_deref().unwrap_or("n/a"));
        out.push('\n');
        out.push_str("From Cache: ");
        out.push_str(match entry.from_cache {
            Some(true) => "yes",
            Some(false) => "no",
            None => "unknown",
        });
        out.push('\n');
        out.push_str("From Prefetch: ");
        out.push_str(match entry.from_prefetch {
            Some(true) => "yes",
            Some(false) => "no",
            None => "unknown",
        });
        out.push('\n');
        if let Some(error_text) = &entry.error_text {
            out.push_str("Error: ");
            out.push_str(error_text);
            out.push('\n');
        }
        out
    }

    pub fn summary_rows(&self) -> Vec<[String; 6]> {
        self.entries()
            .iter()
            .map(|entry| {
                [
                    entry.display_name().to_string(),
                    entry.method.clone(),
                    entry.resource_type.label().to_string(),
                    entry.status.map_or_else(
                        || entry.request_state.label().to_string(),
                        |value| value.to_string(),
                    ),
                    entry
                        .body_size
                        .map_or_else(|| String::from("n/a"), |size| format!("{size} B")),
                    entry.duration_ms.map_or_else(
                        || String::from("n/a"),
                        |duration_ms| format!("{duration_ms} ms"),
                    ),
                ]
            })
            .collect()
    }
}
