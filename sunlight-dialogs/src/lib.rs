#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use sunlight_audio::SystemSound;

pub const WIRE_VERSION: u16 = 2;
pub const LEGACY_WIRE_VERSION: u16 = 1;
pub const WIRE_MAGIC_REQUEST: u32 = 0x5344_5251;
pub const WIRE_MAGIC_RESULT: u32 = 0x5344_5253;
pub const MAX_TEXT_BYTES: usize = 1024;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogKind {
    Alert = 1,
    Confirm = 2,
    TextInput = 3,
    OpenFile = 10,
    OpenFolder = 11,
    SaveFile = 12,
    ColorPicker = 13,
    FontPicker = 14,
    PrintDialog = 15,
}

impl DialogKind {
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Alert),
            2 => Some(Self::Confirm),
            3 => Some(Self::TextInput),
            10 => Some(Self::OpenFile),
            11 => Some(Self::OpenFolder),
            12 => Some(Self::SaveFile),
            13 => Some(Self::ColorPicker),
            14 => Some(Self::FontPicker),
            15 => Some(Self::PrintDialog),
            _ => None,
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmStyle {
    OkCancel = 1,
    YesNo = 2,
}

impl ConfirmStyle {
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::OkCancel),
            2 => Some(Self::YesNo),
            _ => None,
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogButton {
    Ok = 1,
    Cancel = 2,
    Yes = 3,
    No = 4,
}

impl DialogButton {
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Ok),
            2 => Some(Self::Cancel),
            3 => Some(Self::Yes),
            4 => Some(Self::No),
            _ => None,
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogSeverity {
    Information = 0,
    Success = 1,
    Warning = 2,
    Error = 3,
    Critical = 4,
    Question = 5,
}

impl DialogSeverity {
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Information),
            1 => Some(Self::Success),
            2 => Some(Self::Warning),
            3 => Some(Self::Error),
            4 => Some(Self::Critical),
            5 => Some(Self::Question),
            _ => None,
        }
    }

    pub const fn system_sound(self) -> Option<SystemSound> {
        match self {
            Self::Information => None,
            Self::Success => Some(SystemSound::Success),
            Self::Warning => Some(SystemSound::Warning),
            Self::Error => Some(SystemSound::Error),
            Self::Critical => Some(SystemSound::Critical),
            Self::Question => Some(SystemSound::Question),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogCommonOptions {
    pub title: String,
    pub message: String,
    pub severity: DialogSeverity,
    pub silent: bool,
}

impl DialogCommonOptions {
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            severity: DialogSeverity::Information,
            silent: false,
        }
    }

    pub const fn with_severity(mut self, severity: DialogSeverity) -> Self {
        self.severity = severity;
        self
    }

    pub const fn silent(mut self) -> Self {
        self.silent = true;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertRequest {
    pub common: DialogCommonOptions,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfirmRequest {
    pub common: DialogCommonOptions,
    pub style: ConfirmStyle,
    pub default_button: DialogButton,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextInputRequest {
    pub common: DialogCommonOptions,
    pub default_value: String,
    pub allow_empty: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenFileRequest {
    pub title: String,
    pub initial_dir: Option<String>,
    pub allowed_mime_types: Vec<String>,
    pub allowed_extensions: Vec<String>,
    pub allow_multiple: bool,
    pub show_preview: bool,
    pub confirm_button_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenFolderRequest {
    pub title: String,
    pub initial_dir: Option<String>,
    pub confirm_button_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveFileRequest {
    pub title: String,
    pub initial_dir: Option<String>,
    pub suggested_name: Option<String>,
    pub default_extension: Option<String>,
    pub allowed_extensions: Vec<String>,
    pub overwrite_confirm: bool,
    pub confirm_button_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogRequest {
    Alert(AlertRequest),
    Confirm(ConfirmRequest),
    TextInput(TextInputRequest),
    OpenFile(OpenFileRequest),
    OpenFolder(OpenFolderRequest),
    SaveFile(SaveFileRequest),
    ColorPicker,
    FontPicker,
    PrintDialog,
}

impl DialogRequest {
    pub const fn kind(&self) -> DialogKind {
        match self {
            Self::Alert(_) => DialogKind::Alert,
            Self::Confirm(_) => DialogKind::Confirm,
            Self::TextInput(_) => DialogKind::TextInput,
            Self::OpenFile(_) => DialogKind::OpenFile,
            Self::OpenFolder(_) => DialogKind::OpenFolder,
            Self::SaveFile(_) => DialogKind::SaveFile,
            Self::ColorPicker => DialogKind::ColorPicker,
            Self::FontPicker => DialogKind::FontPicker,
            Self::PrintDialog => DialogKind::PrintDialog,
        }
    }

    pub fn alert(
        severity: DialogSeverity,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::Alert(AlertRequest {
            common: DialogCommonOptions::new(title, message).with_severity(severity),
        })
    }

    pub fn warning(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self::alert(DialogSeverity::Warning, title, message)
    }

    pub fn error(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self::alert(DialogSeverity::Error, title, message)
    }

    pub fn critical(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self::alert(DialogSeverity::Critical, title, message)
    }

    pub fn success(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self::alert(DialogSeverity::Success, title, message)
    }

    pub fn system_sound(&self) -> Option<SystemSound> {
        let common = match self {
            Self::Alert(request) => &request.common,
            Self::Confirm(request) => &request.common,
            Self::TextInput(request) => &request.common,
            _ => return None,
        };
        if common.silent {
            None
        } else {
            common.severity.system_sound()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogResult {
    Ok,
    Cancel,
    Yes,
    No,
    TextSubmitted(String),
    Dismissed,
    FileSelected(String),
    FilesSelected(Vec<String>),
    FolderSelected(String),
    SavePathSelected(String),
    Cancelled,
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogError {
    BadRequest,
    TooLarge,
    Unsupported,
    Busy,
    Internal,
    HostUnavailable,
    Corrupt,
}

impl DialogError {
    pub const fn code(self) -> u64 {
        match self {
            Self::BadRequest => 1,
            Self::TooLarge => 2,
            Self::Unsupported => 3,
            Self::Busy => 4,
            Self::Internal => 5,
            Self::HostUnavailable => 6,
            Self::Corrupt => 7,
        }
    }

    pub const fn from_code(value: u64) -> Self {
        match value {
            1 => Self::BadRequest,
            2 => Self::TooLarge,
            3 => Self::Unsupported,
            4 => Self::Busy,
            5 => Self::Internal,
            6 => Self::HostUnavailable,
            7 => Self::Corrupt,
            _ => Self::Internal,
        }
    }
}

#[allow(non_snake_case)]
pub mod DialogMsg {
    pub const SHOW_DIALOG: u64 = 0xD201;
    pub const REPLY: u64 = 0xD2FF;
    pub const ERROR: u64 = 0xD2FE;
}

pub fn encode_request(request: &DialogRequest) -> Vec<u8> {
    let mut out = Vec::new();
    push_u32(&mut out, WIRE_MAGIC_REQUEST);
    push_u16(&mut out, WIRE_VERSION);
    out.push(request.kind() as u8);
    let common = match request {
        DialogRequest::Alert(req) => Some(&req.common),
        DialogRequest::Confirm(req) => Some(&req.common),
        DialogRequest::TextInput(req) => Some(&req.common),
        _ => None,
    };
    out.push(common.map(|value| value.silent as u8).unwrap_or(1));
    out.push(
        common
            .map(|value| value.severity as u8)
            .unwrap_or(DialogSeverity::Information as u8),
    );
    out.push(0);
    match request {
        DialogRequest::Alert(req) => {
            push_u16(&mut out, req.common.title.len() as u16);
            push_u16(&mut out, req.common.message.len() as u16);
            out.extend_from_slice(req.common.title.as_bytes());
            out.extend_from_slice(req.common.message.as_bytes());
        }
        DialogRequest::Confirm(req) => {
            push_u16(&mut out, req.common.title.len() as u16);
            push_u16(&mut out, req.common.message.len() as u16);
            out.push(req.style as u8);
            out.push(req.default_button as u8);
            out.extend_from_slice(req.common.title.as_bytes());
            out.extend_from_slice(req.common.message.as_bytes());
        }
        DialogRequest::TextInput(req) => {
            push_u16(&mut out, req.common.title.len() as u16);
            push_u16(&mut out, req.common.message.len() as u16);
            push_u16(&mut out, req.default_value.len() as u16);
            out.push(req.allow_empty as u8);
            out.push(0);
            out.extend_from_slice(req.common.title.as_bytes());
            out.extend_from_slice(req.common.message.as_bytes());
            out.extend_from_slice(req.default_value.as_bytes());
        }
        DialogRequest::OpenFile(req) => {
            push_u16(&mut out, req.title.len() as u16);
            push_u16(
                &mut out,
                req.initial_dir.as_ref().map(|s| s.len()).unwrap_or(0) as u16,
            );
            push_u16(&mut out, join_csv_len(&req.allowed_mime_types) as u16);
            push_u16(&mut out, join_csv_len(&req.allowed_extensions) as u16);
            out.push(req.allow_multiple as u8);
            out.push(req.show_preview as u8);
            push_u16(
                &mut out,
                req.confirm_button_label
                    .as_ref()
                    .map(|s| s.len())
                    .unwrap_or(0) as u16,
            );
            out.extend_from_slice(req.title.as_bytes());
            push_opt_string(&mut out, req.initial_dir.as_ref());
            push_joined_csv(&mut out, &req.allowed_mime_types);
            push_joined_csv(&mut out, &req.allowed_extensions);
            push_opt_string(&mut out, req.confirm_button_label.as_ref());
        }
        DialogRequest::OpenFolder(req) => {
            push_u16(&mut out, req.title.len() as u16);
            push_u16(
                &mut out,
                req.initial_dir.as_ref().map(|s| s.len()).unwrap_or(0) as u16,
            );
            push_u16(
                &mut out,
                req.confirm_button_label
                    .as_ref()
                    .map(|s| s.len())
                    .unwrap_or(0) as u16,
            );
            out.extend_from_slice(req.title.as_bytes());
            push_opt_string(&mut out, req.initial_dir.as_ref());
            push_opt_string(&mut out, req.confirm_button_label.as_ref());
        }
        DialogRequest::SaveFile(req) => {
            push_u16(&mut out, req.title.len() as u16);
            push_u16(
                &mut out,
                req.initial_dir.as_ref().map(|s| s.len()).unwrap_or(0) as u16,
            );
            push_u16(
                &mut out,
                req.suggested_name.as_ref().map(|s| s.len()).unwrap_or(0) as u16,
            );
            push_u16(
                &mut out,
                req.default_extension.as_ref().map(|s| s.len()).unwrap_or(0) as u16,
            );
            push_u16(&mut out, join_csv_len(&req.allowed_extensions) as u16);
            out.push(req.overwrite_confirm as u8);
            out.push(0);
            push_u16(
                &mut out,
                req.confirm_button_label
                    .as_ref()
                    .map(|s| s.len())
                    .unwrap_or(0) as u16,
            );
            out.extend_from_slice(req.title.as_bytes());
            push_opt_string(&mut out, req.initial_dir.as_ref());
            push_opt_string(&mut out, req.suggested_name.as_ref());
            push_opt_string(&mut out, req.default_extension.as_ref());
            push_joined_csv(&mut out, &req.allowed_extensions);
            push_opt_string(&mut out, req.confirm_button_label.as_ref());
        }
        _ => {}
    }
    out
}

pub fn decode_request(bytes: &[u8]) -> Result<DialogRequest, DialogError> {
    let mut index = 0usize;
    if take_u32(bytes, &mut index).ok_or(DialogError::Corrupt)? != WIRE_MAGIC_REQUEST {
        return Err(DialogError::Corrupt);
    }
    let version = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)?;
    if version != WIRE_VERSION && version != LEGACY_WIRE_VERSION {
        return Err(DialogError::Corrupt);
    }
    let kind = DialogKind::from_u8(take_u8(bytes, &mut index).ok_or(DialogError::Corrupt)?)
        .ok_or(DialogError::Unsupported)?;
    let flags = take_u8(bytes, &mut index).ok_or(DialogError::Corrupt)?;
    let explicit_severity = if version >= 2 {
        let severity =
            DialogSeverity::from_u8(take_u8(bytes, &mut index).ok_or(DialogError::Corrupt)?)
                .ok_or(DialogError::Corrupt)?;
        let _reserved = take_u8(bytes, &mut index).ok_or(DialogError::Corrupt)?;
        Some(severity)
    } else {
        None
    };
    let common_options =
        |title: String, message: String, fallback: DialogSeverity| DialogCommonOptions {
            title,
            message,
            severity: explicit_severity.unwrap_or(fallback),
            silent: flags & 1 != 0,
        };
    match kind {
        DialogKind::Alert => {
            let title_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let message_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let title = take_string(bytes, &mut index, title_len)?;
            let message = take_string(bytes, &mut index, message_len)?;
            Ok(DialogRequest::Alert(AlertRequest {
                common: common_options(title, message, DialogSeverity::Information),
            }))
        }
        DialogKind::Confirm => {
            let title_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let message_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let style =
                ConfirmStyle::from_u8(take_u8(bytes, &mut index).ok_or(DialogError::Corrupt)?)
                    .ok_or(DialogError::Corrupt)?;
            let default_button =
                DialogButton::from_u8(take_u8(bytes, &mut index).ok_or(DialogError::Corrupt)?)
                    .ok_or(DialogError::Corrupt)?;
            let title = take_string(bytes, &mut index, title_len)?;
            let message = take_string(bytes, &mut index, message_len)?;
            Ok(DialogRequest::Confirm(ConfirmRequest {
                common: common_options(title, message, DialogSeverity::Question),
                style,
                default_button,
            }))
        }
        DialogKind::TextInput => {
            let title_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let message_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let default_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let allow_empty = take_u8(bytes, &mut index).ok_or(DialogError::Corrupt)? != 0;
            let _reserved = take_u8(bytes, &mut index).ok_or(DialogError::Corrupt)?;
            let title = take_string(bytes, &mut index, title_len)?;
            let message = take_string(bytes, &mut index, message_len)?;
            let default_value = take_string(bytes, &mut index, default_len)?;
            Ok(DialogRequest::TextInput(TextInputRequest {
                common: common_options(title, message, DialogSeverity::Information),
                default_value,
                allow_empty,
            }))
        }
        DialogKind::OpenFile => {
            let title_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let initial_dir_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let mime_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let ext_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let allow_multiple = take_u8(bytes, &mut index).ok_or(DialogError::Corrupt)? != 0;
            let show_preview = take_u8(bytes, &mut index).ok_or(DialogError::Corrupt)? != 0;
            let confirm_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let title = take_string(bytes, &mut index, title_len)?;
            let initial_dir = take_opt_string(bytes, &mut index, initial_dir_len)?;
            let allowed_mime_types = take_csv_strings(bytes, &mut index, mime_len)?;
            let allowed_extensions = take_csv_strings(bytes, &mut index, ext_len)?;
            let confirm_button_label = take_opt_string(bytes, &mut index, confirm_len)?;
            Ok(DialogRequest::OpenFile(OpenFileRequest {
                title,
                initial_dir,
                allowed_mime_types,
                allowed_extensions,
                allow_multiple,
                show_preview,
                confirm_button_label,
            }))
        }
        DialogKind::OpenFolder => {
            let title_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let initial_dir_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let confirm_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let title = take_string(bytes, &mut index, title_len)?;
            let initial_dir = take_opt_string(bytes, &mut index, initial_dir_len)?;
            let confirm_button_label = take_opt_string(bytes, &mut index, confirm_len)?;
            Ok(DialogRequest::OpenFolder(OpenFolderRequest {
                title,
                initial_dir,
                confirm_button_label,
            }))
        }
        DialogKind::SaveFile => {
            let title_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let initial_dir_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let suggested_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let default_ext_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let allowed_ext_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let overwrite_confirm = take_u8(bytes, &mut index).ok_or(DialogError::Corrupt)? != 0;
            let _reserved = take_u8(bytes, &mut index).ok_or(DialogError::Corrupt)?;
            let confirm_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
            let title = take_string(bytes, &mut index, title_len)?;
            let initial_dir = take_opt_string(bytes, &mut index, initial_dir_len)?;
            let suggested_name = take_opt_string(bytes, &mut index, suggested_len)?;
            let default_extension = take_opt_string(bytes, &mut index, default_ext_len)?;
            let allowed_extensions = take_csv_strings(bytes, &mut index, allowed_ext_len)?;
            let confirm_button_label = take_opt_string(bytes, &mut index, confirm_len)?;
            Ok(DialogRequest::SaveFile(SaveFileRequest {
                title,
                initial_dir,
                suggested_name,
                default_extension,
                allowed_extensions,
                overwrite_confirm,
                confirm_button_label,
            }))
        }
        DialogKind::ColorPicker => Ok(DialogRequest::ColorPicker),
        DialogKind::FontPicker => Ok(DialogRequest::FontPicker),
        DialogKind::PrintDialog => Ok(DialogRequest::PrintDialog),
    }
}

pub fn encode_result(result: &DialogResult) -> Vec<u8> {
    let mut out = Vec::new();
    push_u32(&mut out, WIRE_MAGIC_RESULT);
    push_u16(&mut out, WIRE_VERSION);
    match result {
        DialogResult::Ok => {
            out.push(1);
            out.push(0);
            push_u16(&mut out, 0);
        }
        DialogResult::Cancel => {
            out.push(2);
            out.push(0);
            push_u16(&mut out, 0);
        }
        DialogResult::Yes => {
            out.push(3);
            out.push(0);
            push_u16(&mut out, 0);
        }
        DialogResult::No => {
            out.push(4);
            out.push(0);
            push_u16(&mut out, 0);
        }
        DialogResult::TextSubmitted(text) => {
            out.push(5);
            out.push(0);
            push_u16(&mut out, text.len() as u16);
            out.extend_from_slice(text.as_bytes());
        }
        DialogResult::Dismissed => {
            out.push(6);
            out.push(0);
            push_u16(&mut out, 0);
        }
        DialogResult::FileSelected(path) => {
            out.push(7);
            out.push(0);
            push_u16(&mut out, path.len() as u16);
            out.extend_from_slice(path.as_bytes());
        }
        DialogResult::FilesSelected(paths) => {
            let joined_len = join_csv_len(paths);
            out.push(8);
            out.push(0);
            push_u16(&mut out, joined_len as u16);
            push_joined_csv(&mut out, paths);
        }
        DialogResult::FolderSelected(path) => {
            out.push(9);
            out.push(0);
            push_u16(&mut out, path.len() as u16);
            out.extend_from_slice(path.as_bytes());
        }
        DialogResult::SavePathSelected(path) => {
            out.push(10);
            out.push(0);
            push_u16(&mut out, path.len() as u16);
            out.extend_from_slice(path.as_bytes());
        }
        DialogResult::Cancelled => {
            out.push(11);
            out.push(0);
            push_u16(&mut out, 0);
        }
        DialogResult::Error(message) => {
            out.push(12);
            out.push(0);
            push_u16(&mut out, message.len() as u16);
            out.extend_from_slice(message.as_bytes());
        }
    }
    out
}

pub fn decode_result(bytes: &[u8]) -> Result<DialogResult, DialogError> {
    let mut index = 0usize;
    if take_u32(bytes, &mut index).ok_or(DialogError::Corrupt)? != WIRE_MAGIC_RESULT {
        return Err(DialogError::Corrupt);
    }
    let version = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)?;
    if version != WIRE_VERSION && version != LEGACY_WIRE_VERSION {
        return Err(DialogError::Corrupt);
    }
    let tag = take_u8(bytes, &mut index).ok_or(DialogError::Corrupt)?;
    let _reserved = take_u8(bytes, &mut index).ok_or(DialogError::Corrupt)?;
    let text_len = take_u16(bytes, &mut index).ok_or(DialogError::Corrupt)? as usize;
    Ok(match tag {
        1 => DialogResult::Ok,
        2 => DialogResult::Cancel,
        3 => DialogResult::Yes,
        4 => DialogResult::No,
        5 => DialogResult::TextSubmitted(take_string(bytes, &mut index, text_len)?),
        6 => DialogResult::Dismissed,
        7 => DialogResult::FileSelected(take_string(bytes, &mut index, text_len)?),
        8 => DialogResult::FilesSelected(take_csv_strings(bytes, &mut index, text_len)?),
        9 => DialogResult::FolderSelected(take_string(bytes, &mut index, text_len)?),
        10 => DialogResult::SavePathSelected(take_string(bytes, &mut index, text_len)?),
        11 => DialogResult::Cancelled,
        12 => DialogResult::Error(take_string(bytes, &mut index, text_len)?),
        _ => return Err(DialogError::Corrupt),
    })
}

pub fn validate_request(request: &DialogRequest) -> Result<(), DialogError> {
    match request {
        DialogRequest::Alert(req) => validate_common(&req.common),
        DialogRequest::Confirm(req) => validate_common(&req.common),
        DialogRequest::TextInput(req) => {
            validate_common(&req.common)?;
            if req.default_value.len() > MAX_TEXT_BYTES {
                return Err(DialogError::TooLarge);
            }
            Ok(())
        }
        DialogRequest::OpenFile(req) => {
            if req.title.len() > MAX_TEXT_BYTES {
                return Err(DialogError::TooLarge);
            }
            validate_optional_string(req.initial_dir.as_ref())?;
            validate_string_list(&req.allowed_mime_types)?;
            validate_string_list(&req.allowed_extensions)?;
            validate_optional_string(req.confirm_button_label.as_ref())
        }
        DialogRequest::OpenFolder(req) => {
            if req.title.len() > MAX_TEXT_BYTES {
                return Err(DialogError::TooLarge);
            }
            validate_optional_string(req.initial_dir.as_ref())?;
            validate_optional_string(req.confirm_button_label.as_ref())
        }
        DialogRequest::SaveFile(req) => {
            if req.title.len() > MAX_TEXT_BYTES {
                return Err(DialogError::TooLarge);
            }
            validate_optional_string(req.initial_dir.as_ref())?;
            validate_optional_string(req.suggested_name.as_ref())?;
            validate_optional_string(req.default_extension.as_ref())?;
            validate_string_list(&req.allowed_extensions)?;
            validate_optional_string(req.confirm_button_label.as_ref())
        }
        _ => Err(DialogError::Unsupported),
    }
}

fn validate_common(common: &DialogCommonOptions) -> Result<(), DialogError> {
    if common.title.len() > MAX_TEXT_BYTES || common.message.len() > MAX_TEXT_BYTES {
        return Err(DialogError::TooLarge);
    }
    Ok(())
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_opt_string(out: &mut Vec<u8>, text: Option<&String>) {
    if let Some(text) = text {
        out.extend_from_slice(text.as_bytes());
    }
}

fn take_u8(bytes: &[u8], index: &mut usize) -> Option<u8> {
    let value = *bytes.get(*index)?;
    *index += 1;
    Some(value)
}

fn take_u16(bytes: &[u8], index: &mut usize) -> Option<u16> {
    let slice = bytes.get(*index..(*index + 2))?;
    *index += 2;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn take_u32(bytes: &[u8], index: &mut usize) -> Option<u32> {
    let slice = bytes.get(*index..(*index + 4))?;
    *index += 4;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn take_string(bytes: &[u8], index: &mut usize, len: usize) -> Result<String, DialogError> {
    let slice = bytes
        .get(*index..(*index + len))
        .ok_or(DialogError::Corrupt)?;
    *index += len;
    let text = core::str::from_utf8(slice).map_err(|_| DialogError::Corrupt)?;
    Ok(String::from(text))
}

fn take_opt_string(
    bytes: &[u8],
    index: &mut usize,
    len: usize,
) -> Result<Option<String>, DialogError> {
    if len == 0 {
        return Ok(None);
    }
    Ok(Some(take_string(bytes, index, len)?))
}

fn push_joined_csv(out: &mut Vec<u8>, values: &[String]) {
    for (idx, value) in values.iter().enumerate() {
        if idx != 0 {
            out.push(b',');
        }
        out.extend_from_slice(value.as_bytes());
    }
}

fn join_csv_len(values: &[String]) -> usize {
    values.iter().map(String::len).sum::<usize>() + values.len().saturating_sub(1)
}

fn take_csv_strings(
    bytes: &[u8],
    index: &mut usize,
    len: usize,
) -> Result<Vec<String>, DialogError> {
    if len == 0 {
        return Ok(Vec::new());
    }
    let text = take_string(bytes, index, len)?;
    let mut out = Vec::new();
    for part in text.split(',') {
        if !part.is_empty() {
            out.push(String::from(part));
        }
    }
    Ok(out)
}

fn validate_optional_string(text: Option<&String>) -> Result<(), DialogError> {
    if text.map(|s| s.len()).unwrap_or(0) > MAX_TEXT_BYTES {
        return Err(DialogError::TooLarge);
    }
    Ok(())
}

fn validate_string_list(values: &[String]) -> Result<(), DialogError> {
    for value in values {
        if value.len() > MAX_TEXT_BYTES {
            return Err(DialogError::TooLarge);
        }
    }
    Ok(())
}

pub fn result_keyword(result: &DialogResult) -> &str {
    match result {
        DialogResult::Ok => "ok",
        DialogResult::Cancel => "cancel",
        DialogResult::Yes => "yes",
        DialogResult::No => "no",
        DialogResult::TextSubmitted(_) => "submitted",
        DialogResult::Dismissed => "dismissed",
        DialogResult::FileSelected(_) => "file",
        DialogResult::FilesSelected(_) => "files",
        DialogResult::FolderSelected(_) => "folder",
        DialogResult::SavePathSelected(_) => "save-path",
        DialogResult::Cancelled => "cancelled",
        DialogResult::Error(_) => "error",
    }
}

pub fn confirm_labels(style: ConfirmStyle) -> (&'static str, &'static str) {
    match style {
        ConfirmStyle::OkCancel => ("OK", "Cancel"),
        ConfirmStyle::YesNo => ("Yes", "No"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn alert_roundtrip() {
        let request = DialogRequest::Alert(AlertRequest {
            common: DialogCommonOptions {
                title: String::from("Warning"),
                message: String::from("Something happened"),
                severity: DialogSeverity::Warning,
                silent: false,
            },
        });
        let encoded = encode_request(&request);
        assert_eq!(decode_request(&encoded).unwrap(), request);
    }

    #[test]
    fn input_result_roundtrip() {
        let result = DialogResult::TextSubmitted(String::from("file.txt"));
        let encoded = encode_result(&result);
        assert_eq!(decode_result(&encoded).unwrap(), result);
    }

    #[test]
    fn open_file_request_roundtrip() {
        let request = DialogRequest::OpenFile(OpenFileRequest {
            title: String::from("Open File"),
            initial_dir: Some(String::from("/home/demo")),
            allowed_mime_types: vec![String::from("text/plain"), String::from("image/png")],
            allowed_extensions: vec![String::from("txt"), String::from("png")],
            allow_multiple: true,
            show_preview: false,
            confirm_button_label: Some(String::from("Import")),
        });
        let encoded = encode_request(&request);
        assert_eq!(decode_request(&encoded).unwrap(), request);
    }

    #[test]
    fn save_file_request_roundtrip() {
        let request = DialogRequest::SaveFile(SaveFileRequest {
            title: String::from("Save File"),
            initial_dir: Some(String::from("/tmp")),
            suggested_name: Some(String::from("note")),
            default_extension: Some(String::from("txt")),
            allowed_extensions: vec![String::from("txt"), String::from("md")],
            overwrite_confirm: true,
            confirm_button_label: Some(String::from("Save")),
        });
        let encoded = encode_request(&request);
        assert_eq!(decode_request(&encoded).unwrap(), request);
    }

    #[test]
    fn file_result_roundtrip() {
        let result = DialogResult::FilesSelected(vec![
            String::from("/tmp/a.txt"),
            String::from("/tmp/b.txt"),
        ]);
        let encoded = encode_result(&result);
        assert_eq!(decode_result(&encoded).unwrap(), result);
    }

    #[test]
    fn dialog_severity_maps_once_and_supports_silence() {
        let expected = [
            (DialogSeverity::Information, None),
            (DialogSeverity::Success, Some(SystemSound::Success)),
            (DialogSeverity::Warning, Some(SystemSound::Warning)),
            (DialogSeverity::Error, Some(SystemSound::Error)),
            (DialogSeverity::Critical, Some(SystemSound::Critical)),
            (DialogSeverity::Question, Some(SystemSound::Question)),
        ];
        for (severity, sound) in expected {
            let request = DialogRequest::alert(severity, "Title", "Body");
            assert_eq!(request.system_sound(), sound);
            let silent = match request {
                DialogRequest::Alert(mut alert) => {
                    alert.common.silent = true;
                    DialogRequest::Alert(alert)
                }
                _ => unreachable!(),
            };
            assert_eq!(silent.system_sound(), None);
        }
    }

    #[test]
    fn v2_roundtrip_preserves_sound_policy() {
        let request = DialogRequest::critical("Disk", "Write failed");
        let decoded = decode_request(&encode_request(&request)).unwrap();
        assert_eq!(decoded, request);
        assert_eq!(decoded.system_sound(), Some(SystemSound::Critical));
    }
}
