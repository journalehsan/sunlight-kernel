//! Narrow, presentation-safe Wise Owl conversation transport.
//!
//! The console never creates intents, confirmation grants, or execution
//! requests. It submits bounded user turns and renders only typed public
//! responses.

use alloc::string::String;
use alloc::vec::Vec;

#[cfg(not(test))]
use sunlight_ipc::monotonic_millis;
use sunlight_ipc::{ipc_call_timeout, nameserver_lookup_timeout, shm_alloc, shm_free, IpcMsg};
use wiseowl_brain::native_ipc::{
    BrainIpcHeader, BrainOp, BRAIN_ENDPOINT, BRAIN_IPC_HEADER_LEN, SHM_PAGE_SIZE,
};
use wiseowl_brain::protocol::{
    ConsoleUiCommandWire, ConsoleUiRequestWire, ConsoleUiResponseWire, CONSOLE_UI_MAX_LOCALE_BYTES,
    CONSOLE_UI_MAX_TEXT_BYTES,
};

pub const MAX_TRANSPORT_TEXT_BYTES: usize = CONSOLE_UI_MAX_TEXT_BYTES;
pub const MAX_TRANSPORT_LOCALE_BYTES: usize = CONSOLE_UI_MAX_LOCALE_BYTES;
pub const MAX_TRANSPORT_CHOICES: usize = 8;
const CONSOLE_UI_TIMEOUT_MS: u64 = 500;
const MAX_PENDING_RESPONSES: usize = 16;

#[cfg(test)]
fn now_millis() -> u64 {
    0
}

#[cfg(not(test))]
fn now_millis() -> u64 {
    monotonic_millis()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConversationId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConversationRequestId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationRequirement {
    Soft,
    Strong,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClarificationChoice {
    pub candidate_id: u64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmationPrompt {
    pub description: String,
    pub target: String,
    pub reason: String,
    pub requirement: ConfirmationRequirement,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WiseOwlConversationUiRequest {
    SubmitTurn {
        conversation_id: ConversationId,
        session_id: SessionId,
        request_id: ConversationRequestId,
        locale: String,
        text: String,
        runtime_snapshot_generation: u64,
    },
    SelectClarification {
        conversation_id: ConversationId,
        session_id: SessionId,
        request_id: ConversationRequestId,
        candidate_id: u64,
    },
    SubmitConfirmation {
        conversation_id: ConversationId,
        session_id: SessionId,
        request_id: ConversationRequestId,
        approved: bool,
    },
    CancelPendingAction {
        conversation_id: ConversationId,
        session_id: SessionId,
        request_id: ConversationRequestId,
    },
    QueryConversationState {
        conversation_id: ConversationId,
        session_id: SessionId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionProgressStage {
    RequestUnderstood,
    TargetResolved,
    PolicyAllowed,
    ConfirmationApproved,
    LaunchAccepted,
    WaitingForReadiness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionFailureKind {
    DispatchFailed,
    ExitedEarly,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WiseOwlConversationUiResponse {
    Accepted {
        request_id: ConversationRequestId,
    },
    AssistantText {
        request_id: ConversationRequestId,
        text: String,
    },
    ClarificationRequired {
        request_id: ConversationRequestId,
        prompt: String,
        choices: Vec<ClarificationChoice>,
        expires_at_ms: u64,
    },
    ConfirmationRequired {
        request_id: ConversationRequestId,
        prompt: ConfirmationPrompt,
    },
    ActionProgress {
        request_id: ConversationRequestId,
        stage: ActionProgressStage,
        label: String,
    },
    ActionReady {
        request_id: ConversationRequestId,
        label: String,
    },
    ActionFailed {
        request_id: ConversationRequestId,
        kind: ActionFailureKind,
        label: String,
    },
    Cancelled {
        request_id: ConversationRequestId,
        label: String,
    },
    Rejected {
        request_id: ConversationRequestId,
        code: u16,
        message: String,
    },
    SessionInvalidated,
    Unavailable,
}

pub trait ConversationTransport {
    fn submit(&mut self, request: WiseOwlConversationUiRequest) -> WiseOwlConversationUiResponse;

    fn poll(&mut self) -> Option<WiseOwlConversationUiResponse>;
}

/// Production transport boundary. The service endpoint is intentionally the
/// only route the console probes; no planner, executor, launcher, or MemoryDB
/// object is linked into this crate.
pub struct NativeConversationTransport {
    pending: Vec<WiseOwlConversationUiResponse>,
}

impl NativeConversationTransport {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    fn queue(&mut self, response: WiseOwlConversationUiResponse) {
        if self.pending.len() < MAX_PENDING_RESPONSES {
            self.pending.push(response);
        }
    }

    fn wire_request(
        request: WiseOwlConversationUiRequest,
    ) -> Result<(ConversationRequestId, ConsoleUiRequestWire), WiseOwlConversationUiResponse> {
        match request {
            WiseOwlConversationUiRequest::SubmitTurn {
                conversation_id,
                session_id,
                request_id,
                locale,
                text,
                runtime_snapshot_generation,
            } => {
                let command = ConsoleUiCommandWire::SubmitTurn {
                    locale: heapless_string(&locale, MAX_TRANSPORT_LOCALE_BYTES)
                        .ok_or_else(|| rejection(request_id, 400, "Invalid locale."))?,
                    text: heapless_string(&text, MAX_TRANSPORT_TEXT_BYTES)
                        .ok_or_else(|| rejection(request_id, 400, "Message is too large."))?,
                };
                Ok((
                    request_id,
                    ConsoleUiRequestWire {
                        conversation_id: conversation_id.0,
                        session_id: session_id.0,
                        request_id: request_id.0,
                        runtime_snapshot_generation,
                        command,
                    },
                ))
            }
            WiseOwlConversationUiRequest::SelectClarification {
                conversation_id,
                session_id,
                request_id,
                candidate_id,
            } => Ok((
                request_id,
                ConsoleUiRequestWire {
                    conversation_id: conversation_id.0,
                    session_id: session_id.0,
                    request_id: request_id.0,
                    runtime_snapshot_generation: 0,
                    command: ConsoleUiCommandWire::SelectClarification { candidate_id },
                },
            )),
            WiseOwlConversationUiRequest::SubmitConfirmation {
                conversation_id,
                session_id,
                request_id,
                approved,
            } => Ok((
                request_id,
                ConsoleUiRequestWire {
                    conversation_id: conversation_id.0,
                    session_id: session_id.0,
                    request_id: request_id.0,
                    runtime_snapshot_generation: 0,
                    command: ConsoleUiCommandWire::SubmitConfirmation { approved },
                },
            )),
            WiseOwlConversationUiRequest::CancelPendingAction {
                conversation_id,
                session_id,
                request_id,
            } => Ok((
                request_id,
                ConsoleUiRequestWire {
                    conversation_id: conversation_id.0,
                    session_id: session_id.0,
                    request_id: request_id.0,
                    runtime_snapshot_generation: 0,
                    command: ConsoleUiCommandWire::CancelPendingAction,
                },
            )),
            WiseOwlConversationUiRequest::QueryConversationState {
                conversation_id,
                session_id,
            } => Ok((
                ConversationRequestId(0),
                ConsoleUiRequestWire {
                    conversation_id: conversation_id.0,
                    session_id: session_id.0,
                    request_id: 0,
                    runtime_snapshot_generation: 0,
                    command: ConsoleUiCommandWire::QueryConversationState,
                },
            )),
        }
    }

    fn send(
        &mut self,
        request_id: ConversationRequestId,
        request: ConsoleUiRequestWire,
    ) -> WiseOwlConversationUiResponse {
        let Some(endpoint) = nameserver_lookup_timeout(BRAIN_ENDPOINT, CONSOLE_UI_TIMEOUT_MS)
        else {
            return WiseOwlConversationUiResponse::Unavailable;
        };
        let body = request.encode();
        if body.len() + BRAIN_IPC_HEADER_LEN > SHM_PAGE_SIZE as usize {
            return rejection(request_id, 413, "Message is too large.");
        }
        let (request_ptr, request_cap) = match shm_alloc() {
            Ok(allocation) => allocation,
            Err(_) => return WiseOwlConversationUiResponse::Unavailable,
        };
        let (response_ptr, response_cap) = match shm_alloc() {
            Ok(allocation) => allocation,
            Err(_) => {
                let _ = shm_free(request_cap);
                return WiseOwlConversationUiResponse::Unavailable;
            }
        };
        let header = BrainIpcHeader {
            protocol_version: wiseowl_brain::native_ipc::NATIVE_PROTOCOL_VERSION,
            operation: BrainOp::ConsoleUi.as_u16(),
            flags: 0,
            request_id: request_id.0,
            body_len: body.len() as u32,
            reserved: 0,
        };
        let header_bytes = header.encode();
        unsafe {
            core::ptr::copy_nonoverlapping(
                header_bytes.as_ptr(),
                request_ptr,
                BRAIN_IPC_HEADER_LEN,
            );
            core::ptr::copy_nonoverlapping(
                body.as_ptr(),
                request_ptr.add(BRAIN_IPC_HEADER_LEN),
                body.len(),
            );
        }
        let message = IpcMsg::with_label(BrainOp::ConsoleUi.label())
            .word(0, body.len() as u64)
            .with_cap(0, request_cap)
            .with_cap(1, response_cap);
        let reply = ipc_call_timeout(endpoint, message, CONSOLE_UI_TIMEOUT_MS);
        let _ = shm_free(request_cap);
        let reply = match reply {
            Ok(reply) => reply,
            Err(_) => {
                let _ = shm_free(response_cap);
                return WiseOwlConversationUiResponse::Unavailable;
            }
        };
        let response = take_reply_body(&reply, response_ptr, request_id);
        let _ = shm_free(response_cap);
        let Some(bytes) = response else {
            return WiseOwlConversationUiResponse::Unavailable;
        };
        let Ok((response, _)) = ConsoleUiResponseWire::decode(&bytes) else {
            return WiseOwlConversationUiResponse::Unavailable;
        };
        if response.request_id() != request_id.0 {
            return rejection(
                request_id,
                409,
                "Conversation response did not match the request.",
            );
        }

        match response {
            ConsoleUiResponseWire::AssistantText { request_id, text } => {
                self.queue(WiseOwlConversationUiResponse::AssistantText {
                    request_id: ConversationRequestId(request_id),
                    text: String::from(text.as_str()),
                });
                WiseOwlConversationUiResponse::Accepted {
                    request_id: ConversationRequestId(request_id),
                }
            }
            ConsoleUiResponseWire::Rejected {
                request_id,
                code,
                message,
            } => WiseOwlConversationUiResponse::Rejected {
                request_id: ConversationRequestId(request_id),
                code,
                message: String::from(message.as_str()),
            },
            ConsoleUiResponseWire::Cancelled { request_id, label } => {
                self.queue(WiseOwlConversationUiResponse::Cancelled {
                    request_id: ConversationRequestId(request_id),
                    label: String::from(label.as_str()),
                });
                WiseOwlConversationUiResponse::Accepted {
                    request_id: ConversationRequestId(request_id),
                }
            }
            ConsoleUiResponseWire::Unavailable { .. } => WiseOwlConversationUiResponse::Unavailable,
        }
    }
}

impl ConversationTransport for NativeConversationTransport {
    fn submit(&mut self, request: WiseOwlConversationUiRequest) -> WiseOwlConversationUiResponse {
        let (request_id, request) = match Self::wire_request(request) {
            Ok(request) => request,
            Err(response) => return response,
        };
        self.send(request_id, request)
    }

    fn poll(&mut self) -> Option<WiseOwlConversationUiResponse> {
        (!self.pending.is_empty()).then(|| self.pending.remove(0))
    }
}

fn heapless_string<const N: usize>(text: &str, max_len: usize) -> Option<heapless::String<N>> {
    if text.len() > max_len {
        return None;
    }
    heapless::String::try_from(text).ok()
}

fn rejection(
    request_id: ConversationRequestId,
    code: u16,
    message: &str,
) -> WiseOwlConversationUiResponse {
    WiseOwlConversationUiResponse::Rejected {
        request_id,
        code,
        message: String::from(message),
    }
}

fn take_reply_body(
    reply: &IpcMsg,
    response_ptr: *mut u8,
    request_id: ConversationRequestId,
) -> Option<Vec<u8>> {
    if reply.label != BrainOp::Reply.label() || reply.cap_count != 0 || reply.word_count != 1 {
        return None;
    }
    let page =
        unsafe { core::slice::from_raw_parts(response_ptr as *const u8, SHM_PAGE_SIZE as usize) };
    let header = BrainIpcHeader::decode(page).ok()?;
    if header.operation != BrainOp::Reply.as_u16()
        || header.request_id != request_id.0
        || header.body_len as u64 != reply.words[0]
    {
        return None;
    }
    let body_end = BRAIN_IPC_HEADER_LEN.checked_add(header.body_len as usize)?;
    (body_end <= page.len()).then(|| page[BRAIN_IPC_HEADER_LEN..body_end].to_vec())
}

#[cfg(test)]
mod native_reply_tests {
    use super::*;

    #[test]
    fn caller_owned_reply_buffer_requires_matching_request_id() {
        let response = ConsoleUiResponseWire::assistant_text(17, "Local reply");
        let body = response.encode();
        let header = BrainIpcHeader {
            protocol_version: wiseowl_brain::native_ipc::NATIVE_PROTOCOL_VERSION,
            operation: BrainOp::Reply.as_u16(),
            flags: 0,
            request_id: 17,
            body_len: body.len() as u32,
            reserved: 0,
        };
        let mut page = [0u8; SHM_PAGE_SIZE as usize];
        page[..BRAIN_IPC_HEADER_LEN].copy_from_slice(&header.encode());
        page[BRAIN_IPC_HEADER_LEN..BRAIN_IPC_HEADER_LEN + body.len()].copy_from_slice(&body);
        let reply = IpcMsg::with_label(BrainOp::Reply.label()).word(0, body.len() as u64);

        assert_eq!(
            take_reply_body(&reply, page.as_mut_ptr(), ConversationRequestId(17)),
            Some(Vec::from(body.as_slice()))
        );
        assert_eq!(
            take_reply_body(&reply, page.as_mut_ptr(), ConversationRequestId(18)),
            None
        );
    }
}

#[cfg(any(test, feature = "conversation-v1-test"))]
pub struct FakeConversationTransport {
    pending: Vec<WiseOwlConversationUiResponse>,
    available: bool,
}

#[cfg(any(test, feature = "conversation-v1-test"))]
impl FakeConversationTransport {
    pub const fn new() -> Self {
        Self {
            pending: Vec::new(),
            available: true,
        }
    }

    pub fn offline(mut self) -> Self {
        self.available = false;
        self
    }

    fn queue(&mut self, response: WiseOwlConversationUiResponse) {
        if self.pending.len() < 16 {
            self.pending.push(response);
        }
    }

    fn response_text(text: &str) -> String {
        String::from(text)
    }
}

#[cfg(any(test, feature = "conversation-v1-test"))]
impl Default for FakeConversationTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "conversation-v1-test"))]
impl ConversationTransport for FakeConversationTransport {
    fn submit(&mut self, request: WiseOwlConversationUiRequest) -> WiseOwlConversationUiResponse {
        if !self.available {
            return WiseOwlConversationUiResponse::Unavailable;
        }

        match request {
            WiseOwlConversationUiRequest::SubmitTurn {
                request_id, text, ..
            } => {
                let normalized = text.to_ascii_lowercase();
                if normalized.contains("offline") {
                    self.available = false;
                    self.queue(WiseOwlConversationUiResponse::Unavailable);
                } else if normalized.contains("settings") || text.contains("تنظیمات") {
                    self.queue(WiseOwlConversationUiResponse::ClarificationRequired {
                        request_id,
                        prompt: Self::response_text("Which settings page should I open?"),
                        choices: Vec::from([
                            ClarificationChoice {
                                candidate_id: 41,
                                label: Self::response_text("Display Settings"),
                            },
                            ClarificationChoice {
                                candidate_id: 42,
                                label: Self::response_text("Network Settings"),
                            },
                        ]),
                        expires_at_ms: now_millis().saturating_add(30_000),
                    });
                } else if normalized.contains("confirm") {
                    self.queue(WiseOwlConversationUiResponse::ConfirmationRequired {
                        request_id,
                        prompt: ConfirmationPrompt {
                            description: Self::response_text("Open Calculator"),
                            target: Self::response_text("Calculator"),
                            reason: Self::response_text("This action opens an application."),
                            requirement: ConfirmationRequirement::Soft,
                            expires_at_ms: now_millis().saturating_add(30_000),
                        },
                    });
                } else if normalized.contains("timeout") {
                    self.queue(WiseOwlConversationUiResponse::ActionProgress {
                        request_id,
                        stage: ActionProgressStage::LaunchAccepted,
                        label: Self::response_text("Opening Display Settings…"),
                    });
                    self.queue(WiseOwlConversationUiResponse::ActionProgress {
                        request_id,
                        stage: ActionProgressStage::WaitingForReadiness,
                        label: Self::response_text("Waiting for application readiness…"),
                    });
                    self.queue(WiseOwlConversationUiResponse::ActionFailed {
                        request_id,
                        kind: ActionFailureKind::TimedOut,
                        label: Self::response_text(
                            "Display Settings did not become ready in time.",
                        ),
                    });
                } else {
                    self.queue(WiseOwlConversationUiResponse::AssistantText {
                        request_id,
                        text: Self::response_text("I understood your request."),
                    });
                }
                WiseOwlConversationUiResponse::Accepted { request_id }
            }
            WiseOwlConversationUiRequest::SelectClarification {
                request_id,
                candidate_id,
                ..
            } => {
                if candidate_id != 41 && candidate_id != 42 {
                    return WiseOwlConversationUiResponse::Rejected {
                        request_id,
                        code: 1002,
                        message: Self::response_text(
                            "That clarification choice is no longer available.",
                        ),
                    };
                }
                self.queue(WiseOwlConversationUiResponse::ActionProgress {
                    request_id,
                    stage: ActionProgressStage::LaunchAccepted,
                    label: Self::response_text("Opening Display Settings…"),
                });
                self.queue(WiseOwlConversationUiResponse::ActionProgress {
                    request_id,
                    stage: ActionProgressStage::WaitingForReadiness,
                    label: Self::response_text("Waiting for application readiness…"),
                });
                self.queue(WiseOwlConversationUiResponse::ActionReady {
                    request_id,
                    label: Self::response_text("Display Settings is ready."),
                });
                WiseOwlConversationUiResponse::Accepted { request_id }
            }
            WiseOwlConversationUiRequest::SubmitConfirmation {
                request_id,
                approved,
                ..
            } => {
                if !approved {
                    self.queue(WiseOwlConversationUiResponse::Cancelled {
                        request_id,
                        label: Self::response_text("Action cancelled."),
                    });
                } else {
                    self.queue(WiseOwlConversationUiResponse::ActionProgress {
                        request_id,
                        stage: ActionProgressStage::ConfirmationApproved,
                        label: Self::response_text("Confirmation approved."),
                    });
                    self.queue(WiseOwlConversationUiResponse::ActionReady {
                        request_id,
                        label: Self::response_text("Calculator is ready."),
                    });
                }
                WiseOwlConversationUiResponse::Accepted { request_id }
            }
            WiseOwlConversationUiRequest::CancelPendingAction { request_id, .. } => {
                self.queue(WiseOwlConversationUiResponse::Cancelled {
                    request_id,
                    label: Self::response_text("Action cancelled."),
                });
                WiseOwlConversationUiResponse::Accepted { request_id }
            }
            WiseOwlConversationUiRequest::QueryConversationState { .. } => {
                WiseOwlConversationUiResponse::Accepted {
                    request_id: ConversationRequestId(0),
                }
            }
        }
    }

    fn poll(&mut self) -> Option<WiseOwlConversationUiResponse> {
        (!self.pending.is_empty()).then(|| self.pending.remove(0))
    }
}
