use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use crate::resources::request::{
    format_body_size, format_duration_ms, format_header_name, NetworkRequestEntry,
    PageNetworkSession, RequestState, ResourcePriority, ResourceType,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkPaneFocus {
    RequestList,
    Details,
}

impl Default for NetworkPaneFocus {
    fn default() -> Self {
        Self::RequestList
    }
}

#[derive(Debug, Clone)]
pub struct NetworkTabState {
    session: PageNetworkSession,
    list_scroll_offset: usize,
    detail_scroll_offset: usize,
    selected_request_id: Option<u64>,
    focused_pane: NetworkPaneFocus,
    detail_text_cache: String,
    detail_text_dirty: bool,
    summary_rows_cache: Vec<[String; 6]>,
    summary_rows_dirty: bool,
}

impl Default for NetworkTabState {
    fn default() -> Self {
        Self {
            session: PageNetworkSession::default(),
            list_scroll_offset: 0,
            detail_scroll_offset: 0,
            selected_request_id: None,
            focused_pane: NetworkPaneFocus::RequestList,
            detail_text_cache: String::new(),
            detail_text_dirty: true,
            summary_rows_cache: Vec::new(),
            summary_rows_dirty: true,
        }
    }
}

impl NetworkTabState {
    pub fn clear_for_new_page(&mut self) {
        self.session.clear();
        self.list_scroll_offset = 0;
        self.detail_scroll_offset = 0;
        self.selected_request_id = None;
        self.focused_pane = NetworkPaneFocus::RequestList;
        self.detail_text_cache.clear();
        self.detail_text_dirty = true;
        self.summary_rows_cache.clear();
        self.summary_rows_dirty = true;
    }

    pub fn begin_main_document_request(
        &mut self,
        method: &str,
        resource_url: String,
        request_headers: Vec<(String, String)>,
    ) -> u64 {
        self.begin_request(
            method,
            resource_url,
            request_headers,
            ResourceType::MainDocument,
            ResourcePriority::MainDocument,
            RequestState::Queued,
        )
    }

    pub fn begin_request(
        &mut self,
        method: &str,
        resource_url: String,
        request_headers: Vec<(String, String)>,
        resource_type: ResourceType,
        priority: ResourcePriority,
        request_state: RequestState,
    ) -> u64 {
        let request_id = self.session.begin_request(
            method,
            resource_url,
            request_headers,
            resource_type,
            priority,
            request_state,
        );
        if self.selected_request_id.is_none() {
            self.selected_request_id = Some(request_id);
        }
        self.invalidate_caches();
        request_id
    }

    pub fn set_request_state(&mut self, request_id: u64, request_state: RequestState) {
        self.session.set_request_state(request_id, request_state);
        self.invalidate_caches();
    }

    pub fn set_stylesheet_metadata(
        &mut self,
        request_id: u64,
        initiator: Option<String>,
        import_depth: usize,
        parse_success: Option<bool>,
        imported_rule_count: Option<usize>,
    ) {
        self.session.set_stylesheet_metadata(
            request_id,
            initiator,
            import_depth,
            parse_success,
            imported_rule_count,
        );
        self.invalidate_caches();
    }

    pub fn complete_request(
        &mut self,
        request_id: u64,
        final_url: Option<String>,
        status_code: u16,
        status_text: String,
        duration_ms: Option<u64>,
        body_size: Option<usize>,
        content_type: Option<String>,
        response_headers: Vec<(String, String)>,
        response_body: Option<Vec<u8>>,
        from_cache: Option<bool>,
        from_prefetch: Option<bool>,
    ) {
        self.session.complete_request(
            request_id,
            final_url,
            status_code,
            status_text,
            duration_ms,
            body_size,
            content_type,
            response_headers,
            response_body,
            from_cache,
            from_prefetch,
        );
        self.invalidate_caches();
    }

    pub fn fail_request(
        &mut self,
        request_id: u64,
        final_url: Option<String>,
        status_code: Option<u16>,
        status_text: Option<String>,
        error_text: impl Into<String>,
    ) {
        self.session
            .fail_request(request_id, final_url, status_code, status_text, error_text);
        self.invalidate_caches();
    }

    pub fn entries(&self) -> &[NetworkRequestEntry] {
        self.session.entries()
    }

    pub fn request_id_at_row(&self, row_index: usize) -> Option<u64> {
        self.session.request_id_at(row_index)
    }

    pub fn list_scroll_offset(&self) -> usize {
        self.list_scroll_offset
    }

    pub fn set_list_scroll_offset(&mut self, scroll_offset: usize) {
        self.list_scroll_offset = scroll_offset;
    }

    pub fn detail_scroll_offset(&self) -> usize {
        self.detail_scroll_offset
    }

    pub fn set_detail_scroll_offset(&mut self, scroll_offset: usize) {
        self.detail_scroll_offset = scroll_offset;
    }

    pub fn selected_request_id(&self) -> Option<u64> {
        self.selected_request_id
    }

    pub fn selected_row(&self) -> Option<usize> {
        self.selected_request_id
            .and_then(|request_id| self.session.index_of(request_id))
    }

    pub fn selected_entry(&self) -> Option<&NetworkRequestEntry> {
        self.selected_request_id
            .and_then(|request_id| self.session.entry(request_id))
    }

    pub fn select_request(&mut self, request_id: Option<u64>) {
        let next = request_id.filter(|request_id| self.session.entry(*request_id).is_some());
        if next != self.selected_request_id {
            self.detail_scroll_offset = 0;
            self.detail_text_dirty = true;
        }
        self.selected_request_id = next;
    }

    pub fn focused_pane(&self) -> NetworkPaneFocus {
        self.focused_pane
    }

    pub fn set_focused_pane(&mut self, focused_pane: NetworkPaneFocus) {
        self.focused_pane = focused_pane;
    }

    pub fn selected_request_detail_text(&mut self) -> &str {
        if !self.detail_text_dirty {
            return self.detail_text_cache.as_str();
        }
        let Some(entry) = self.selected_entry() else {
            self.detail_text_cache = String::from("Select a request to inspect its details.");
            self.detail_text_dirty = false;
            return self.detail_text_cache.as_str();
        };

        let mut out = String::new();
        out.push_str("General\n");
        out.push_str("-------\n");
        push_field(&mut out, "Request URL", entry.request_url());
        push_field(&mut out, "Final URL", entry.final_url_or_requested());
        push_field(&mut out, "Request Method", entry.method.as_str());
        push_field(&mut out, "Status Code", entry.status_display().as_str());
        push_field(
            &mut out,
            "Content Type",
            entry.content_type.as_deref().unwrap_or("n/a"),
        );
        push_field(
            &mut out,
            "Transferred/Body Size",
            entry.response_body_size_display().as_str(),
        );
        push_field(&mut out, "Duration", entry.duration_display().as_str());
        if entry.resource_type == ResourceType::Stylesheet {
            push_field(
                &mut out,
                "Initiator",
                entry.initiator.as_deref().unwrap_or("document"),
            );
            push_field(
                &mut out,
                "Import Depth",
                &entry.import_depth.unwrap_or(0).to_string(),
            );
            push_field(
                &mut out,
                "Parse Success",
                match entry.parse_success {
                    Some(true) => "yes",
                    Some(false) => "no",
                    None => "pending",
                },
            );
            push_field(
                &mut out,
                "Imported Rules",
                &entry
                    .imported_rule_count
                    .map_or_else(|| String::from("pending"), |count| count.to_string()),
            );
        }
        out.push('\n');

        out.push_str("Request Headers\n");
        out.push_str("---------------\n");
        write_headers_section(
            &mut out,
            &entry.request_headers,
            "No request headers were captured for this request.",
        );
        out.push('\n');

        out.push_str("Response Headers\n");
        out.push_str("----------------\n");
        write_headers_section(
            &mut out,
            &entry.response_headers,
            "No response headers were captured for this request.",
        );

        if let Some(error_text) = &entry.error_text {
            out.push('\n');
            out.push_str("Error\n");
            out.push_str("-----\n");
            out.push_str(error_text);
            out.push('\n');
        }
        self.detail_text_cache = out;
        self.detail_text_dirty = false;
        self.detail_text_cache.as_str()
    }

    pub fn summary_rows(&mut self) -> &[[String; 6]] {
        if !self.summary_rows_dirty {
            return self.summary_rows_cache.as_slice();
        }
        self.summary_rows_cache = self
            .entries()
            .iter()
            .map(|entry| {
                [
                    entry.display_name().to_string(),
                    entry.method.clone(),
                    entry.status_display(),
                    entry.resource_type.label().to_string(),
                    format_body_size(entry.response_body_size)
                        .trim_end_matches(" bytes")
                        .to_string(),
                    format_duration_ms(entry.duration_ms),
                ]
            })
            .collect();
        self.summary_rows_dirty = false;
        self.summary_rows_cache.as_slice()
    }

    fn invalidate_caches(&mut self) {
        self.detail_text_dirty = true;
        self.summary_rows_dirty = true;
    }
}

fn push_field(out: &mut String, label: &str, value: &str) {
    out.push_str(label);
    out.push_str(": ");
    out.push_str(value);
    out.push('\n');
}

fn write_headers_section(out: &mut String, headers: &[(String, String)], empty_state: &str) {
    if headers.is_empty() {
        out.push_str(empty_state);
        out.push('\n');
        return;
    }

    for (name, value) in headers {
        out.push_str(&format_header_name(name));
        out.push_str(": ");
        out.push_str(value);
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_request_is_selected_automatically() {
        let mut state = NetworkTabState::default();
        let request_id = state.begin_main_document_request(
            "GET",
            String::from("https://example.com/"),
            Vec::new(),
        );
        assert_eq!(state.selected_request_id(), Some(request_id));
        assert_eq!(state.selected_row(), Some(0));
    }

    #[test]
    fn detail_text_reports_truthful_request_header_empty_state() {
        let mut state = NetworkTabState::default();
        let request_id = state.begin_main_document_request(
            "GET",
            String::from("https://example.com/"),
            Vec::new(),
        );
        state.complete_request(
            request_id,
            Some(String::from("https://example.com/final")),
            200,
            String::from("OK"),
            Some(84),
            Some(512),
            Some(String::from("text/html")),
            vec![(String::from("content-type"), String::from("text/html"))],
            None,
            Some(false),
            Some(false),
        );

        let detail = state.selected_request_detail_text();
        assert!(detail.contains("No request headers were captured for this request."));
        assert!(detail.contains("Content-Type: text/html"));
        assert!(detail.contains("Final URL: https://example.com/final"));
    }

    #[test]
    fn summary_and_detail_views_are_stable_between_idle_frames() {
        let mut state = NetworkTabState::default();
        state.begin_main_document_request("GET", String::from("https://example.com/"), Vec::new());
        let rows_ptr = state.summary_rows().as_ptr();
        let detail_ptr = state.selected_request_detail_text().as_ptr();
        for _ in 0..100 {
            assert_eq!(state.summary_rows().as_ptr(), rows_ptr);
            assert_eq!(state.selected_request_detail_text().as_ptr(), detail_ptr);
        }
    }
}
