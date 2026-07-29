use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

#[cfg(not(test))]
use sunlight_ipc::monotonic_millis;
use sunlight_ui::{
    widgets::{
        Button, ButtonState, ConversationBubble, ConversationBubbleKind, OwlAvatar, OwlAvatarState,
        Panel, TextInput,
    },
    Canvas, Event, Point, Rect, Theme, VecText,
};

use crate::transport::{
    ActionFailureKind, ActionProgressStage, ClarificationChoice, ConfirmationPrompt,
    ConfirmationRequirement, ConversationId, ConversationRequestId, ConversationTransport,
    NativeConversationTransport, SessionId, WiseOwlConversationUiRequest,
    WiseOwlConversationUiResponse, MAX_TRANSPORT_CHOICES, MAX_TRANSPORT_TEXT_BYTES,
};
use crate::ui::{FONT_UI_MEDIUM, FONT_UI_SMALL};

pub const MAX_INPUT_BYTES: usize = MAX_TRANSPORT_TEXT_BYTES;
pub const MAX_INPUT_SCALARS: usize = 2048;
pub const MAX_VISIBLE_MESSAGES: usize = 128;
const MAX_UI_AUDIT_EVENTS: usize = 96;
const INPUT_HEIGHT: u32 = 30;
const SEND_WIDTH: u32 = 72;
const KEY_ENTER: u8 = 0x1C;
const KEY_PAGE_UP: u8 = 0x49;
const KEY_PAGE_DOWN: u8 = 0x51;

#[cfg(test)]
fn now_millis() -> u64 {
    0
}

#[cfg(not(test))]
fn now_millis() -> u64 {
    monotonic_millis()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatSubmissionState {
    Idle,
    Validating,
    Sending,
    AwaitingResponse,
    AwaitingClarification,
    AwaitingConfirmation,
    AwaitingOutcome,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextDirection {
    Ltr,
    Rtl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiAuditKind {
    InputFocused,
    SubmissionAttempted,
    SubmissionAccepted,
    SubmissionRejected,
    ClarificationSelected,
    ConfirmationSubmitted,
    CancellationSubmitted,
    TypedEventReceived,
    MalformedEventRejected,
    SessionInvalidated,
    ConnectionLost,
    ConnectionRestored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedactedUiAuditEvent {
    pub kind: UiAuditKind,
    pub request_id: Option<ConversationRequestId>,
    pub text_len: u16,
}

#[derive(Debug, Clone)]
pub enum Message {
    User {
        request_id: ConversationRequestId,
        text: String,
    },
    Assistant(String),
    SystemStatus(String),
    ActionResult(String),
    Error(String),
}

#[derive(Debug, Clone)]
pub struct ActionProgressData {
    pub request_id: ConversationRequestId,
    pub stage: ActionProgressStage,
    pub label: String,
}

#[derive(Debug, Clone)]
enum PendingInteractionView {
    Clarification {
        request_id: ConversationRequestId,
        prompt: String,
        choices: Vec<ClarificationChoice>,
        expires_at_ms: u64,
        submitting: bool,
    },
    Confirmation {
        request_id: ConversationRequestId,
        prompt: ConfirmationPrompt,
        submitting: bool,
    },
}

#[derive(Debug, Clone, Copy)]
enum CardAction {
    Clarification(u64),
    Confirm,
    Cancel,
}

pub struct ConversationPage {
    messages: Vec<Message>,
    input: TextInput<'static, MAX_INPUT_BYTES>,
    owl: OwlAvatar,
    transport: Box<dyn ConversationTransport>,
    conversation_id: ConversationId,
    session_id: Option<SessionId>,
    app_instance_id: u32,
    submission_sequence: u32,
    submission_state: ChatSubmissionState,
    pending_card: Option<PendingInteractionView>,
    current_action: Option<ActionProgressData>,
    send_rect: Rect,
    card_actions: Vec<(Rect, CardAction)>,
    scroll_offset: usize,
    auto_scroll: bool,
    new_messages_pending: bool,
    offline: bool,
    status: Option<String>,
    seen_events: Vec<(ConversationRequestId, u8)>,
    audit: Vec<RedactedUiAuditEvent>,
}

impl ConversationPage {
    pub fn new() -> Self {
        Self::new_with_transport(Box::new(NativeConversationTransport::new()))
    }

    pub fn new_with_transport(transport: Box<dyn ConversationTransport>) -> Self {
        Self {
            messages: Vec::new(),
            input: TextInput::new(Rect::new(0, 0, 0, 0))
                .with_font(&FONT_UI_MEDIUM)
                .with_placeholder("Type a message…")
                .with_clipboard_source(b"wiseowl-console"),
            owl: OwlAvatar::new(Rect::new(0, 0, 0, 0)),
            transport,
            conversation_id: ConversationId(1),
            session_id: Some(SessionId(1)),
            app_instance_id: now_millis() as u32 ^ 0x5749_5345,
            submission_sequence: 0,
            submission_state: ChatSubmissionState::Idle,
            pending_card: None,
            current_action: None,
            send_rect: Rect::new(0, 0, 0, 0),
            card_actions: Vec::new(),
            scroll_offset: 0,
            auto_scroll: true,
            new_messages_pending: false,
            offline: false,
            status: None,
            seen_events: Vec::new(),
            audit: Vec::new(),
        }
    }

    pub fn draw(&mut self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        let panel = Panel::with_title(rect, "Conversation").with_font(&FONT_UI_MEDIUM);
        panel.draw(canvas, theme);
        let content = panel.content_rect().inset(10);
        let input_y = content.bottom() - INPUT_HEIGHT as i32;
        self.send_rect = Rect::new(
            content.right() - SEND_WIDTH as i32,
            input_y,
            SEND_WIDTH,
            INPUT_HEIGHT,
        );
        self.input.rect = Rect::new(
            content.x,
            input_y,
            content.w.saturating_sub(SEND_WIDTH + 8),
            INPUT_HEIGHT,
        );

        let history = Rect::new(
            content.x,
            content.y,
            content.w,
            (input_y - content.y - 8).max(0) as u32,
        );
        self.draw_history(canvas, theme, history);

        self.input.draw(canvas, theme);
        let send_state = if self.can_submit() {
            ButtonState::Normal
        } else {
            ButtonState::Disabled
        };
        Button::new(self.send_rect, "Send")
            .with_font(&FONT_UI_SMALL)
            .with_state(send_state)
            .draw(canvas, theme);

        if let Some(status) = &self.status {
            FONT_UI_SMALL.draw(
                canvas,
                status,
                content.x,
                input_y - 16,
                if self.offline {
                    theme.warn
                } else {
                    theme.text_dim
                },
            );
        }
        if self.new_messages_pending {
            FONT_UI_SMALL.draw(
                canvas,
                "New messages below",
                content.right() - 116,
                input_y - 16,
                theme.accent,
            );
        }

        self.owl.rect = Rect::new(content.right() - 52, content.y + 4, 42, 42);
        self.owl.state = self.owl_state();
        self.owl.draw(canvas, theme);
    }

    pub fn update(&mut self, event: Event) -> bool {
        if matches!(event, Event::Tick) {
            self.owl.advance();
            let expired = self.expire_pending_card();
            return self.poll_transport() || expired;
        }

        if let Event::Click { x, y } = event {
            if self.handle_card_click(Point::new(x, y)) {
                return true;
            }
            if self.send_rect.contains(Point::new(x, y)) {
                return self.submit_input();
            }
        }

        if let Event::KeyPress {
            keycode,
            pressed: true,
            ..
        } = event
        {
            if keycode == KEY_ENTER && self.input.active {
                return self.submit_input();
            }
            if keycode == KEY_PAGE_UP {
                return self.scroll_by(-6);
            }
            if keycode == KEY_PAGE_DOWN {
                return self.scroll_by(6);
            }
        }

        if matches!(event, Event::Key('\n' | '\r')) && self.input.active {
            return self.submit_input();
        }

        let previous = String::from(self.input.value());
        let was_active = self.input.active;
        let changed = self.input.update(event);
        if self.input.value().chars().count() > MAX_INPUT_SCALARS {
            self.input.set_text(&previous);
            self.status = Some(String::from("Message is too long."));
            return true;
        }
        if matches!(event, Event::Key(ch) if !ch.is_control())
            && previous == self.input.value()
            && self.input.value().len() >= MAX_INPUT_BYTES
        {
            self.status = Some(String::from("Message is too long."));
        }
        if !was_active && self.input.active {
            self.record_audit(UiAuditKind::InputFocused, None, 0);
        }
        changed
    }

    pub fn invalidate_session(&mut self) {
        self.session_id = None;
        self.pending_card = None;
        self.current_action = None;
        self.input.set_text("");
        self.submission_state = ChatSubmissionState::Failed;
        self.status = Some(String::from("Your session has ended."));
        self.push_message(Message::SystemStatus(String::from("Session ended.")));
        self.record_audit(UiAuditKind::SessionInvalidated, None, 0);
    }

    pub fn input_text(&self) -> &str {
        self.input.value()
    }

    pub fn input_direction(&self) -> TextDirection {
        direction_for_text(self.input.value())
    }

    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    pub fn submission_state(&self) -> ChatSubmissionState {
        self.submission_state
    }

    pub fn audit_events(&self) -> &[RedactedUiAuditEvent] {
        &self.audit
    }

    pub fn has_pending_interaction(&self) -> bool {
        self.pending_card.is_some()
    }

    fn draw_history(&mut self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        self.card_actions.clear();
        let bubble_width = rect.w.min(560);
        let visible_rows = (rect.h / 36).max(1) as usize;
        let start = if self.auto_scroll {
            self.messages.len().saturating_sub(visible_rows)
        } else {
            self.scroll_offset
                .min(self.messages.len().saturating_sub(1))
        };
        let mut y = rect.y;

        for message in self.messages.iter().skip(start) {
            let (text, kind, user) = match message {
                Message::User { text, .. } => (text.as_str(), ConversationBubbleKind::User, true),
                Message::Assistant(text) => {
                    (text.as_str(), ConversationBubbleKind::Assistant, false)
                }
                Message::SystemStatus(text) | Message::ActionResult(text) => {
                    (text.as_str(), ConversationBubbleKind::System, false)
                }
                Message::Error(text) => (text.as_str(), ConversationBubbleKind::Error, false),
            };
            let rtl = direction_for_text(text) == TextDirection::Rtl;
            let x = if user || rtl {
                rect.right() - bubble_width as i32
            } else {
                rect.x
            };
            let bubble = ConversationBubble::new(Rect::new(x, y, bubble_width, 30), text, kind)
                .with_font(&FONT_UI_SMALL);
            bubble.draw(canvas, theme);
            y += bubble.preferred_height() as i32 + 6;
            if y >= rect.bottom() {
                return;
            }
        }

        if let Some(progress) = &self.current_action {
            let bubble = ConversationBubble::new(
                Rect::new(rect.x, y, bubble_width, 30),
                &progress.label,
                ConversationBubbleKind::Progress,
            )
            .with_font(&FONT_UI_SMALL);
            bubble.draw(canvas, theme);
            y += bubble.preferred_height() as i32 + 6;
        }
        if y < rect.bottom() {
            self.draw_pending_card(canvas, theme, rect, y, bubble_width);
        }
    }

    fn draw_pending_card(
        &mut self,
        canvas: &mut Canvas,
        theme: &Theme,
        rect: Rect,
        mut y: i32,
        width: u32,
    ) {
        let Some(card) = self.pending_card.clone() else {
            return;
        };
        match &card {
            PendingInteractionView::Clarification {
                prompt,
                choices,
                submitting,
                ..
            } => {
                ConversationBubble::new(
                    Rect::new(rect.x, y, width, 30),
                    prompt,
                    ConversationBubbleKind::Clarification,
                )
                .with_font(&FONT_UI_SMALL)
                .draw(canvas, theme);
                y += 36;
                for choice in choices {
                    let action_rect = Rect::new(rect.x + 8, y, width.saturating_sub(16), 26);
                    Button::secondary(action_rect, &choice.label)
                        .with_font(&FONT_UI_SMALL)
                        .with_state(if *submitting {
                            ButtonState::Disabled
                        } else {
                            ButtonState::Normal
                        })
                        .draw(canvas, theme);
                    self.card_actions
                        .push((action_rect, CardAction::Clarification(choice.candidate_id)));
                    y += 30;
                }
                self.draw_cancel_button(canvas, theme, rect.x + 8, y, *submitting);
            }
            PendingInteractionView::Confirmation {
                prompt, submitting, ..
            } => {
                ConversationBubble::new(
                    Rect::new(rect.x, y, width, ConversationBubble::DETAIL_HEIGHT),
                    &prompt.description,
                    ConversationBubbleKind::Confirmation,
                )
                .with_detail(&prompt.reason)
                .with_font(&FONT_UI_SMALL)
                .draw(canvas, theme);
                y += ConversationBubble::DETAIL_HEIGHT as i32 + 6;
                let can_confirm =
                    matches!(prompt.requirement, ConfirmationRequirement::Soft) && !*submitting;
                let confirm = Rect::new(rect.x + 8, y, 86, 26);
                Button::new(
                    confirm,
                    if can_confirm {
                        "Confirm"
                    } else {
                        "Trusted proof"
                    },
                )
                .with_font(&FONT_UI_SMALL)
                .with_state(if can_confirm {
                    ButtonState::Normal
                } else {
                    ButtonState::Disabled
                })
                .draw(canvas, theme);
                if can_confirm {
                    self.card_actions.push((confirm, CardAction::Confirm));
                }
                self.draw_cancel_button(canvas, theme, confirm.right() + 6, y, *submitting);
            }
        }
    }

    fn draw_cancel_button(
        &mut self,
        canvas: &mut Canvas,
        theme: &Theme,
        x: i32,
        y: i32,
        submitting: bool,
    ) {
        let cancel = Rect::new(x, y, 76, 26);
        Button::secondary(cancel, "Cancel")
            .with_font(&FONT_UI_SMALL)
            .with_state(if submitting {
                ButtonState::Disabled
            } else {
                ButtonState::Normal
            })
            .draw(canvas, theme);
        if !submitting {
            self.card_actions.push((cancel, CardAction::Cancel));
        }
    }

    fn handle_card_click(&mut self, point: Point) -> bool {
        let action = self
            .card_actions
            .iter()
            .find_map(|(rect, action)| rect.contains(point).then_some(*action));
        match action {
            Some(CardAction::Clarification(candidate_id)) => {
                self.select_clarification(candidate_id)
            }
            Some(CardAction::Confirm) => self.submit_confirmation(true),
            Some(CardAction::Cancel) => self.cancel_pending(),
            None => false,
        }
    }

    fn submit_input(&mut self) -> bool {
        if self.offline || self.session_id.is_none() || !self.can_submit() {
            return false;
        }
        self.submission_state = ChatSubmissionState::Validating;
        let text = String::from(self.input.value());
        let Some(session_id) = self.session_id else {
            return false;
        };
        let request_id = self.next_request_id();
        self.record_audit(
            UiAuditKind::SubmissionAttempted,
            Some(request_id),
            text.len(),
        );
        self.submission_state = ChatSubmissionState::Sending;
        let response = self
            .transport
            .submit(WiseOwlConversationUiRequest::SubmitTurn {
                conversation_id: self.conversation_id,
                session_id,
                request_id,
                locale: locale_for_text(&text),
                text: text.clone(),
                runtime_snapshot_generation: 0,
            });
        match response {
            WiseOwlConversationUiResponse::Accepted { .. } => {
                self.push_message(Message::User { request_id, text });
                self.input.set_text("");
                self.status = Some(String::from("Thinking…"));
                self.submission_state = ChatSubmissionState::AwaitingResponse;
                self.record_audit(UiAuditKind::SubmissionAccepted, Some(request_id), 0);
                true
            }
            response => {
                self.apply_response(response);
                false
            }
        }
    }

    fn select_clarification(&mut self, candidate_id: u64) -> bool {
        let Some(session_id) = self.session_id else {
            return false;
        };
        let Some(PendingInteractionView::Clarification {
            request_id: card_request_id,
            choices,
            submitting,
            ..
        }) = self.pending_card.as_ref()
        else {
            return false;
        };
        if *submitting {
            return false;
        }
        if !choices
            .iter()
            .any(|choice| choice.candidate_id == candidate_id)
        {
            self.record_audit(
                UiAuditKind::MalformedEventRejected,
                Some(*card_request_id),
                0,
            );
            return false;
        }
        let request_id = self.next_request_id();
        if let Some(PendingInteractionView::Clarification { submitting, .. }) =
            self.pending_card.as_mut()
        {
            *submitting = true;
        }
        self.record_audit(UiAuditKind::ClarificationSelected, Some(request_id), 0);
        let response = self
            .transport
            .submit(WiseOwlConversationUiRequest::SelectClarification {
                conversation_id: self.conversation_id,
                session_id,
                request_id,
                candidate_id,
            });
        self.handle_pending_submit_response(response, ChatSubmissionState::AwaitingOutcome)
    }

    fn submit_confirmation(&mut self, approved: bool) -> bool {
        let Some(session_id) = self.session_id else {
            return false;
        };
        let Some(PendingInteractionView::Confirmation {
            prompt, submitting, ..
        }) = self.pending_card.as_ref()
        else {
            return false;
        };
        if *submitting || !matches!(prompt.requirement, ConfirmationRequirement::Soft) {
            return false;
        }
        let request_id = self.next_request_id();
        if let Some(PendingInteractionView::Confirmation { submitting, .. }) =
            self.pending_card.as_mut()
        {
            *submitting = true;
        }
        self.record_audit(UiAuditKind::ConfirmationSubmitted, Some(request_id), 0);
        let response = self
            .transport
            .submit(WiseOwlConversationUiRequest::SubmitConfirmation {
                conversation_id: self.conversation_id,
                session_id,
                request_id,
                approved,
            });
        self.handle_pending_submit_response(response, ChatSubmissionState::AwaitingOutcome)
    }

    fn cancel_pending(&mut self) -> bool {
        let Some(session_id) = self.session_id else {
            return false;
        };
        if self.pending_card.is_none() && self.current_action.is_none() {
            return false;
        }
        let request_id = self.next_request_id();
        self.record_audit(UiAuditKind::CancellationSubmitted, Some(request_id), 0);
        let response = self
            .transport
            .submit(WiseOwlConversationUiRequest::CancelPendingAction {
                conversation_id: self.conversation_id,
                session_id,
                request_id,
            });
        self.handle_pending_submit_response(response, ChatSubmissionState::AwaitingOutcome)
    }

    fn handle_pending_submit_response(
        &mut self,
        response: WiseOwlConversationUiResponse,
        state: ChatSubmissionState,
    ) -> bool {
        match response {
            WiseOwlConversationUiResponse::Accepted { .. } => {
                self.submission_state = state;
                self.status = Some(String::from("Thinking…"));
                true
            }
            response => {
                self.apply_response(response);
                false
            }
        }
    }

    fn poll_transport(&mut self) -> bool {
        let Some(response) = self.transport.poll() else {
            return false;
        };
        self.apply_response(response);
        true
    }

    fn expire_pending_card(&mut self) -> bool {
        let expired = match self.pending_card.as_ref() {
            Some(PendingInteractionView::Clarification { expires_at_ms, .. }) => {
                now_millis() >= *expires_at_ms
            }
            Some(PendingInteractionView::Confirmation { prompt, .. }) => {
                now_millis() >= prompt.expires_at_ms
            }
            None => false,
        };
        if expired {
            self.pending_card = None;
            self.push_message(Message::Error(String::from("The pending action expired.")));
            self.status = None;
            self.submission_state = ChatSubmissionState::Failed;
        }
        expired
    }

    fn apply_response(&mut self, response: WiseOwlConversationUiResponse) {
        let event_key = response_key(&response);
        if let Some(key) = event_key {
            if self.seen_events.contains(&key) {
                return;
            }
            if self.seen_events.len() == MAX_VISIBLE_MESSAGES {
                self.seen_events.remove(0);
            }
            self.seen_events.push(key);
            self.record_audit(UiAuditKind::TypedEventReceived, Some(key.0), 0);
        }
        match response {
            WiseOwlConversationUiResponse::Accepted { .. } => {}
            WiseOwlConversationUiResponse::AssistantText { text, .. } => {
                self.push_message(Message::Assistant(text));
                self.pending_card = None;
                self.current_action = None;
                self.status = None;
                self.submission_state = ChatSubmissionState::Idle;
            }
            WiseOwlConversationUiResponse::ClarificationRequired {
                request_id,
                prompt,
                choices,
                expires_at_ms,
            } => {
                if choices.is_empty() || choices.len() > MAX_TRANSPORT_CHOICES {
                    self.push_message(Message::Error(String::from(
                        "The clarification response was invalid.",
                    )));
                    self.record_audit(UiAuditKind::MalformedEventRejected, Some(request_id), 0);
                    self.submission_state = ChatSubmissionState::Failed;
                    return;
                }
                self.pending_card = Some(PendingInteractionView::Clarification {
                    request_id,
                    prompt,
                    choices,
                    expires_at_ms,
                    submitting: false,
                });
                self.status = Some(String::from("Choose an option."));
                self.submission_state = ChatSubmissionState::AwaitingClarification;
            }
            WiseOwlConversationUiResponse::ConfirmationRequired { request_id, prompt } => {
                self.pending_card = Some(PendingInteractionView::Confirmation {
                    request_id,
                    prompt,
                    submitting: false,
                });
                self.status = Some(String::from("Confirmation required."));
                self.submission_state = ChatSubmissionState::AwaitingConfirmation;
            }
            WiseOwlConversationUiResponse::ActionProgress {
                request_id,
                stage,
                label,
            } => {
                self.pending_card = None;
                self.current_action = Some(ActionProgressData {
                    request_id,
                    stage,
                    label,
                });
                self.status = Some(String::from(match stage {
                    ActionProgressStage::LaunchAccepted => "Opening application…",
                    ActionProgressStage::WaitingForReadiness => "Waiting for readiness…",
                    _ => "Working…",
                }));
                self.submission_state = ChatSubmissionState::AwaitingOutcome;
            }
            WiseOwlConversationUiResponse::ActionReady { label, .. } => {
                self.current_action = None;
                self.pending_card = None;
                self.push_message(Message::ActionResult(label));
                self.status = None;
                self.submission_state = ChatSubmissionState::Idle;
            }
            WiseOwlConversationUiResponse::ActionFailed { kind, label, .. } => {
                self.current_action = None;
                self.pending_card = None;
                let suffix = match kind {
                    ActionFailureKind::DispatchFailed => "Dispatch failed.",
                    ActionFailureKind::ExitedEarly => "Application exited before readiness.",
                    ActionFailureKind::TimedOut => "Outcome timed out.",
                };
                self.push_message(Message::Error(format_message(&label, suffix)));
                self.status = None;
                self.submission_state = ChatSubmissionState::Failed;
            }
            WiseOwlConversationUiResponse::Cancelled { label, .. } => {
                self.current_action = None;
                self.pending_card = None;
                self.push_message(Message::ActionResult(label));
                self.status = None;
                self.submission_state = ChatSubmissionState::Idle;
            }
            WiseOwlConversationUiResponse::Rejected {
                request_id,
                code,
                message,
            } => {
                self.push_message(Message::Error(format_rejection(code, &message)));
                self.status = Some(String::from("Message was not sent."));
                self.submission_state = ChatSubmissionState::Failed;
                self.record_audit(UiAuditKind::SubmissionRejected, Some(request_id), 0);
            }
            WiseOwlConversationUiResponse::SessionInvalidated => self.invalidate_session(),
            WiseOwlConversationUiResponse::Unavailable => {
                self.offline = true;
                self.status = Some(String::from("Wise Owl is offline."));
                self.submission_state = ChatSubmissionState::Failed;
                self.record_audit(UiAuditKind::ConnectionLost, None, 0);
            }
        }
    }

    fn push_message(&mut self, message: Message) {
        if self.messages.len() == MAX_VISIBLE_MESSAGES {
            self.messages.remove(0);
            self.scroll_offset = self.scroll_offset.saturating_sub(1);
        }
        self.messages.push(message);
        if self.auto_scroll {
            self.scroll_offset = self.messages.len().saturating_sub(1);
        } else {
            self.new_messages_pending = true;
        }
    }

    fn can_submit(&self) -> bool {
        self.session_id.is_some()
            && !self.offline
            && matches!(
                self.submission_state,
                ChatSubmissionState::Idle | ChatSubmissionState::Failed
            )
            && valid_input(self.input.value())
    }

    fn scroll_by(&mut self, delta: isize) -> bool {
        let max = self.messages.len().saturating_sub(1);
        let old = self.scroll_offset;
        if delta < 0 {
            self.scroll_offset = self.scroll_offset.saturating_sub(delta.unsigned_abs());
        } else {
            self.scroll_offset = (self.scroll_offset + delta as usize).min(max);
        }
        self.auto_scroll = self.scroll_offset >= max;
        if self.auto_scroll {
            self.new_messages_pending = false;
        }
        old != self.scroll_offset
    }

    fn next_request_id(&mut self) -> ConversationRequestId {
        self.submission_sequence = self.submission_sequence.wrapping_add(1);
        ConversationRequestId(
            ((self.app_instance_id as u64) << 32) | self.submission_sequence as u64,
        )
    }

    fn owl_state(&self) -> OwlAvatarState {
        if self.offline {
            OwlAvatarState::Offline
        } else if self.input.active {
            OwlAvatarState::Listening
        } else {
            match self.submission_state {
                ChatSubmissionState::AwaitingResponse => OwlAvatarState::Thinking,
                ChatSubmissionState::AwaitingClarification => OwlAvatarState::Clarification,
                ChatSubmissionState::AwaitingConfirmation => OwlAvatarState::Confirmation,
                ChatSubmissionState::AwaitingOutcome => {
                    match self.current_action.as_ref().map(|p| p.stage) {
                        Some(ActionProgressStage::LaunchAccepted) => OwlAvatarState::Acting,
                        Some(ActionProgressStage::WaitingForReadiness) => OwlAvatarState::Observing,
                        _ => OwlAvatarState::Thinking,
                    }
                }
                ChatSubmissionState::Failed => OwlAvatarState::Warning,
                _ => OwlAvatarState::Idle,
            }
        }
    }

    fn record_audit(
        &mut self,
        kind: UiAuditKind,
        request_id: Option<ConversationRequestId>,
        text_len: usize,
    ) {
        if self.audit.len() == MAX_UI_AUDIT_EVENTS {
            self.audit.remove(0);
        }
        self.audit.push(RedactedUiAuditEvent {
            kind,
            request_id,
            text_len: text_len.min(u16::MAX as usize) as u16,
        });
    }
}

fn valid_input(text: &str) -> bool {
    !text.trim().is_empty()
        && text.len() <= MAX_INPUT_BYTES
        && text.chars().count() <= MAX_INPUT_SCALARS
}

fn locale_for_text(text: &str) -> String {
    String::from(if direction_for_text(text) == TextDirection::Rtl {
        "fa"
    } else {
        "en"
    })
}

pub fn direction_for_text(text: &str) -> TextDirection {
    text.chars()
        .find(|ch| !ch.is_whitespace())
        .map(|ch| {
            if ('\u{0590}'..='\u{08ff}').contains(&ch) {
                TextDirection::Rtl
            } else {
                TextDirection::Ltr
            }
        })
        .unwrap_or(TextDirection::Ltr)
}

fn response_key(response: &WiseOwlConversationUiResponse) -> Option<(ConversationRequestId, u8)> {
    match response {
        WiseOwlConversationUiResponse::AssistantText { request_id, .. } => Some((*request_id, 1)),
        WiseOwlConversationUiResponse::ClarificationRequired { request_id, .. } => {
            Some((*request_id, 2))
        }
        WiseOwlConversationUiResponse::ConfirmationRequired { request_id, .. } => {
            Some((*request_id, 3))
        }
        WiseOwlConversationUiResponse::ActionProgress {
            request_id, stage, ..
        } => Some((*request_id, 10 + *stage as u8)),
        WiseOwlConversationUiResponse::ActionReady { request_id, .. } => Some((*request_id, 20)),
        WiseOwlConversationUiResponse::ActionFailed {
            request_id, kind, ..
        } => Some((*request_id, 30 + *kind as u8)),
        WiseOwlConversationUiResponse::Cancelled { request_id, .. } => Some((*request_id, 40)),
        WiseOwlConversationUiResponse::Rejected { request_id, .. } => Some((*request_id, 50)),
        _ => None,
    }
}

fn format_message(primary: &str, suffix: &str) -> String {
    let mut result = String::from(primary);
    if !primary.is_empty() {
        result.push(' ');
    }
    result.push_str(suffix);
    result
}

fn format_rejection(code: u16, message: &str) -> String {
    let mut result = String::from("Request rejected (");
    let mut digits = [0u8; 5];
    let mut value = code;
    let mut index = digits.len();
    loop {
        index = index.saturating_sub(1);
        digits[index] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    result.push_str(core::str::from_utf8(&digits[index..]).unwrap_or("0"));
    result.push_str("): ");
    result.push_str(message);
    result
}

trait ButtonExt: Sized {
    fn with_state(self, state: ButtonState) -> Self;
}

impl<'a> ButtonExt for Button<'a> {
    fn with_state(mut self, state: ButtonState) -> Self {
        self.state = state;
        self
    }
}

#[cfg(any(test, feature = "conversation-v1-test"))]
pub fn run_deterministic_gate() -> bool {
    use crate::transport::FakeConversationTransport;

    let mut page = ConversationPage::new_with_transport(Box::new(FakeConversationTransport::new()));
    page.input.rect = Rect::new(0, 0, 300, 30);
    let _ = page.update(Event::Click { x: 4, y: 4 });
    if !page.input.active {
        return false;
    }
    for ch in "hello".chars() {
        let _ = page.update(Event::Key(ch));
    }
    if page.input_text() != "hello" || !page.submit_input() {
        return false;
    }
    let _ = page.update(Event::Tick);
    if page.message_count() != 2 {
        return false;
    }

    for text in ["status", "perfect"] {
        page.input.set_text(text);
        if !page.submit_input() {
            return false;
        }
        let _ = page.update(Event::Tick);
    }
    if page.message_count() != 6 || page.submission_state != ChatSubmissionState::Idle {
        return false;
    }

    page.input.set_text("تنظیمات را باز کن");
    if page.input_direction() != TextDirection::Rtl || !page.submit_input() {
        return false;
    }
    let _ = page.update(Event::Tick);
    if !page.has_pending_interaction() {
        return false;
    }
    if !page.select_clarification(41) {
        return false;
    }
    let _ = page.update(Event::Tick);
    let _ = page.update(Event::Tick);
    let _ = page.update(Event::Tick);

    page.input.set_text("confirm calculator");
    if !page.submit_input() {
        return false;
    }
    let _ = page.update(Event::Tick);
    if !page.cancel_pending() {
        return false;
    }
    let _ = page.update(Event::Tick);

    page.input.set_text("confirm calculator");
    if !page.submit_input() {
        return false;
    }
    let _ = page.update(Event::Tick);
    if !page.submit_confirmation(true) {
        return false;
    }
    let _ = page.update(Event::Tick);
    let _ = page.update(Event::Tick);

    page.input.set_text("timeout");
    if !page.submit_input() {
        return false;
    }
    let _ = page.update(Event::Tick);
    let _ = page.update(Event::Tick);
    let _ = page.update(Event::Tick);
    page.input.set_text("offline");
    if !page.submit_input() {
        return false;
    }
    let _ = page.update(Event::Tick);
    page.input
        .set_text("x".repeat(MAX_INPUT_SCALARS + 1).as_str());
    !page.can_submit()
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;
    use crate::transport::FakeConversationTransport;

    fn page() -> ConversationPage {
        let mut page =
            ConversationPage::new_with_transport(Box::new(FakeConversationTransport::new()));
        page.input.rect = Rect::new(0, 0, 300, 30);
        page
    }

    #[test]
    fn editable_input_preserves_english_persian_and_mixed_unicode() {
        let mut page = page();
        assert!(page.update(Event::Click { x: 8, y: 8 }));
        for ch in "hello سلام".chars() {
            page.update(Event::Key(ch));
        }
        assert_eq!(page.input_text(), "hello سلام");
        assert_eq!(page.input_direction(), TextDirection::Ltr);
        page.input.set_text("سلام hello");
        assert_eq!(page.input_direction(), TextDirection::Rtl);
    }

    #[test]
    fn backspace_is_utf8_safe_and_empty_submission_is_rejected() {
        let mut page = page();
        page.update(Event::Click { x: 8, y: 8 });
        page.update(Event::Key('س'));
        page.update(Event::Key('ل'));
        page.update(Event::Key('\u{8}'));
        assert_eq!(page.input_text(), "س");
        page.input.set_text("  ");
        assert!(!page.submit_input());
    }

    #[test]
    fn accepted_user_message_is_rendered_once_and_response_is_typed() {
        let mut page = page();
        page.input.set_text("hello");
        assert!(page.submit_input());
        assert_eq!(page.message_count(), 1);
        page.update(Event::Tick);
        assert_eq!(page.message_count(), 2);
        page.update(Event::Tick);
        assert_eq!(page.message_count(), 2);
    }

    #[test]
    fn consecutive_turns_have_distinct_request_ids_and_all_render() {
        let mut page = page();
        let mut request_ids = Vec::new();

        for text in ["thanks", "hi", "perfect"] {
            page.input.set_text(text);
            assert!(page.submit_input());
            request_ids.push(match page.messages.last() {
                Some(Message::User { request_id, .. }) => *request_id,
                _ => panic!("accepted turn must add a user message"),
            });
            page.update(Event::Tick);
        }

        assert_eq!(request_ids.len(), 3);
        assert_ne!(request_ids[0], request_ids[1]);
        assert_ne!(request_ids[1], request_ids[2]);
        assert_ne!(request_ids[0], request_ids[2]);
        assert_eq!(page.message_count(), 6);
        assert_eq!(page.submission_state(), ChatSubmissionState::Idle);
    }

    #[test]
    fn clarification_uses_canonical_choice_and_prevents_double_submit() {
        let mut page = page();
        page.input.set_text("open settings");
        assert!(page.submit_input());
        page.update(Event::Tick);
        assert!(page.has_pending_interaction());
        assert!(page.select_clarification(41));
        assert!(!page.select_clarification(41));
    }

    #[test]
    fn strong_confirmation_is_not_downgraded() {
        let mut page = page();
        page.pending_card = Some(PendingInteractionView::Confirmation {
            request_id: ConversationRequestId(7),
            prompt: ConfirmationPrompt {
                description: String::from("Sensitive action"),
                target: String::from("Target"),
                reason: String::from("Reason"),
                requirement: ConfirmationRequirement::Strong,
                expires_at_ms: 10,
            },
            submitting: false,
        });
        assert!(!page.submit_confirmation(true));
    }

    #[test]
    fn bounds_and_message_eviction_are_bounded() {
        let mut page = page();
        page.input
            .set_text("x".repeat(MAX_INPUT_SCALARS + 1).as_str());
        assert!(!page.can_submit());
        for index in 0..MAX_VISIBLE_MESSAGES + 4 {
            page.push_message(Message::Assistant(index.to_string()));
        }
        assert_eq!(page.message_count(), MAX_VISIBLE_MESSAGES);
    }

    #[test]
    fn deterministic_gate_covers_progress_timeout_and_offline() {
        assert!(run_deterministic_gate());
    }
}
