#![no_std]
#![no_main]
#![allow(static_mut_refs)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use sun_font::{
    draw_text as sf_draw, line_height as sf_lh, measure_text as sf_measure, FontRole, TextStyle,
};
use sunlight_audiod::AudioClient;
use sunlight_dialogs::{
    confirm_labels, decode_request, encode_result, validate_request, ConfirmStyle, DialogButton,
    DialogError, DialogMsg, DialogRequest, DialogResult, DialogSeverity, TextInputRequest,
};
use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv_timeout, ipc_reply, nameserver_register, process_yield,
    shm_alloc, shm_free, shm_map, CapabilityToken, IpcMsg, ProcessExit, SHM_PAGE,
};
use sunlight_libc::{self as libc, env, sun_open, DirEntry, FT_DIR, FT_FILE, MAX_PATH};
use sunlight_ui::image::{mime_icon, TgaImage};
use sunlight_ui::widgets::{Button, ButtonState, TextInput};
use sunlight_ui::{
    request_close, App, Canvas, Color, Event, Point, Rect, Theme, UiSymbol, Window, WindowConfig,
    WindowDecoration,
};

const ALERT_W: u32 = 460;
const ALERT_H: u32 = 240;
const INPUT_H: u32 = 284;
const FILE_W: u32 = 760;
const FILE_H: u32 = 520;
const PAD: i32 = 18;
const BTN_W: u32 = 112;
const BTN_H: u32 = 32;
const TEXT_INPUT_H: u32 = 34;
const RADIUS: u32 = 10;
const ROW_H: i32 = 34;
const HEADER_H: i32 = 34;
const STATUS_H: i32 = 22;
const LIST_MAX: usize = 96;
const INFO_LINES: usize = 3;
const KEY_ESC: u8 = 0x01;
const KEY_ENTER: u8 = 0x1C;
const KEY_TAB: u8 = 0x0F;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_LEFT: u8 = 0x4B;
const KEY_BACKSPACE: char = '\u{8}';
const SERVICE_POLL_MS: u64 = 20;

static ICON_FOLDER_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/places/16/folder.tga");
static ICON_INODE_DIRECTORY_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/inode-directory.tga");
static ICON_IMAGE_FILE_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/image-x-generic.tga");
static ICON_TEXT_PLAIN_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/text-plain.tga");
static ICON_TEXT_MARKDOWN_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/text-markdown.tga");
static ICON_TEXT_RUST_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/text-rust.tga");
static ICON_TEXT_GENERIC_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/text-x-generic.tga");
static ICON_APPLICATION_JSON_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/application-json.tga");
static ICON_APPLICATION_EXECUTABLE_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/application-x-executable.tga");
static ICON_APPLICATION_OCTET_STREAM_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/application-octet-stream.tga");
static ICON_AUDIO_GENERIC_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/audio-x-generic.tga");
static ICON_VIDEO_GENERIC_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/video-x-generic.tga");
static ICON_UNKNOWN_FILE_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/unknown.tga");

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 1024 * 1024] = [0; 1024 * 1024];
        static mut NEXT: usize = 0;
        let start = NEXT;
        let align = layout.align();
        let aligned = (start + align - 1) & !(align - 1);
        let end = aligned + layout.size();
        if end > HEAP.len() {
            return core::ptr::null_mut();
        }
        NEXT = end;
        HEAP.as_mut_ptr().add(aligned)
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

#[derive(Clone, Copy)]
struct MimeIconTheme {
    folder: Option<TgaImage>,
    inode_directory: Option<TgaImage>,
    image_generic: Option<TgaImage>,
    text_plain: Option<TgaImage>,
    text_markdown: Option<TgaImage>,
    text_rust: Option<TgaImage>,
    text_generic: Option<TgaImage>,
    application_json: Option<TgaImage>,
    application_executable: Option<TgaImage>,
    application_octet_stream: Option<TgaImage>,
    audio_generic: Option<TgaImage>,
    video_generic: Option<TgaImage>,
    unknown: Option<TgaImage>,
}

impl MimeIconTheme {
    fn load() -> Self {
        Self {
            folder: TgaImage::parse(ICON_FOLDER_TGA).ok(),
            inode_directory: TgaImage::parse(ICON_INODE_DIRECTORY_TGA).ok(),
            image_generic: TgaImage::parse(ICON_IMAGE_FILE_TGA).ok(),
            text_plain: TgaImage::parse(ICON_TEXT_PLAIN_TGA).ok(),
            text_markdown: TgaImage::parse(ICON_TEXT_MARKDOWN_TGA).ok(),
            text_rust: TgaImage::parse(ICON_TEXT_RUST_TGA).ok(),
            text_generic: TgaImage::parse(ICON_TEXT_GENERIC_TGA).ok(),
            application_json: TgaImage::parse(ICON_APPLICATION_JSON_TGA).ok(),
            application_executable: TgaImage::parse(ICON_APPLICATION_EXECUTABLE_TGA).ok(),
            application_octet_stream: TgaImage::parse(ICON_APPLICATION_OCTET_STREAM_TGA).ok(),
            audio_generic: TgaImage::parse(ICON_AUDIO_GENERIC_TGA).ok(),
            video_generic: TgaImage::parse(ICON_VIDEO_GENERIC_TGA).ok(),
            unknown: TgaImage::parse(ICON_UNKNOWN_FILE_TGA).ok(),
        }
    }

    fn icon_for_entry(&self, entry: &DirEntry) -> Option<TgaImage> {
        if entry.file_type == FT_DIR {
            return self
                .folder
                .or(self.inode_directory)
                .or(self.application_octet_stream);
        }
        let mime = sun_open::mime_from_path(entry.name_bytes());
        let mut exact_name = [0u8; mime_icon::MAX_MIME_ICON_NAME];
        let lookup = mime_icon::resolve_file_icon(mime, &mut exact_name);
        lookup
            .exact
            .and_then(|name| self.icon_by_name(name))
            .or_else(|| lookup.family.and_then(|name| self.icon_by_name(name)))
            .or_else(|| self.icon_by_name(lookup.generic))
            .or(self.unknown)
    }

    fn icon_by_name(&self, name: &str) -> Option<TgaImage> {
        match name {
            "folder" => self.folder,
            "inode-directory" => self.inode_directory,
            "image-x-generic" => self.image_generic,
            "text-plain" => self.text_plain,
            "text-markdown" => self.text_markdown,
            "text-rust" => self.text_rust,
            "text-x-generic" => self.text_generic,
            "application-json" => self.application_json,
            "application-x-executable" => self.application_executable,
            "application-octet-stream" => self.application_octet_stream,
            "audio-x-generic" => self.audio_generic,
            "video-x-generic" => self.video_generic,
            "unknown" => self.unknown,
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
struct PathBuf {
    buf: [u8; MAX_PATH],
    len: usize,
}

impl PathBuf {
    const fn root() -> Self {
        let mut buf = [0u8; MAX_PATH];
        buf[0] = b'/';
        Self { buf, len: 1 }
    }

    fn from_str(text: &str) -> Option<Self> {
        let mut out = Self::root();
        if out.set(text) {
            Some(out)
        } else {
            None
        }
    }

    fn set(&mut self, text: &str) -> bool {
        let bytes = text.as_bytes();
        let mut start = 0usize;
        let mut end = bytes.len();
        while start < end && bytes[start].is_ascii_whitespace() {
            start += 1;
        }
        while end > start && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        if start >= end {
            self.buf[0] = b'/';
            self.len = 1;
            return true;
        }
        self.buf[0] = b'/';
        let mut written = 1usize;
        let mut i = start;
        let mut saw_component = false;
        while i < end {
            while i < end && bytes[i] == b'/' {
                i += 1;
            }
            let comp_start = i;
            while i < end && bytes[i] != b'/' {
                i += 1;
            }
            if i > comp_start {
                if saw_component && written < MAX_PATH {
                    self.buf[written] = b'/';
                    written += 1;
                }
                saw_component = true;
                for &b in &bytes[comp_start..i] {
                    if written >= MAX_PATH || b == 0 {
                        return false;
                    }
                    self.buf[written] = b;
                    written += 1;
                }
            }
        }
        self.len = written.max(1);
        true
    }

    fn join(&self, component: &str) -> Option<Self> {
        let bytes = component.as_bytes();
        if bytes.is_empty() || bytes.contains(&0) || bytes.contains(&b'/') {
            return None;
        }
        let mut out = *self;
        if out.len > 1 && out.buf[out.len - 1] != b'/' {
            if out.len >= MAX_PATH {
                return None;
            }
            out.buf[out.len] = b'/';
            out.len += 1;
        }
        if out.len + bytes.len() > MAX_PATH {
            return None;
        }
        out.buf[out.len..out.len + bytes.len()].copy_from_slice(bytes);
        out.len += bytes.len();
        Some(out)
    }

    fn parent(&self) -> Option<Self> {
        if self.len <= 1 {
            return None;
        }
        let mut end = self.len;
        while end > 1 && self.buf[end - 1] == b'/' {
            end -= 1;
        }
        while end > 1 && self.buf[end - 1] != b'/' {
            end -= 1;
        }
        if end <= 1 {
            Some(Self::root())
        } else {
            let mut out = *self;
            out.len = end - 1;
            Some(out)
        }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("/")
    }
}

#[derive(Clone, Copy)]
struct FileRow {
    entry: DirEntry,
    selectable: bool,
}

impl FileRow {
    const fn empty() -> Self {
        Self {
            entry: DirEntry::zeroed(),
            selectable: false,
        }
    }

    fn name_str(&self) -> &str {
        core::str::from_utf8(self.entry.name_bytes()).unwrap_or("?")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileDialogKind {
    OpenFile,
    OpenFolder,
    SaveFile,
}

struct FileDialogState {
    kind: FileDialogKind,
    title: String,
    confirm_label: String,
    current_dir: PathBuf,
    selected: Option<usize>,
    hover_row: Option<usize>,
    scroll_offset: usize,
    rows: [FileRow; LIST_MAX],
    row_count: usize,
    status: String,
    details: [String; INFO_LINES],
    show_preview: bool,
    input: TextInput<'static, MAX_PATH>,
    input_active: bool,
    multi_select_requested: bool,
    allowed_mime_types: Vec<String>,
    allowed_extensions: Vec<String>,
    default_extension: Option<String>,
    overwrite_confirm: bool,
    pending_overwrite_confirm: bool,
}

impl FileDialogState {
    fn from_request(request: &DialogRequest) -> Self {
        let home = detect_home_path();
        let mut current_dir = home;
        let mut title = String::from("Open");
        let mut confirm_label = String::from("Open");
        let mut input = TextInput::new(Rect::new(PAD, 0, FILE_W - (PAD as u32 * 2), TEXT_INPUT_H));
        let mut kind = FileDialogKind::OpenFile;
        let mut show_preview = true;
        let mut multi_select_requested = false;
        let mut allowed_mime_types = Vec::new();
        let mut allowed_extensions = Vec::new();
        let mut default_extension = None;
        let mut overwrite_confirm = false;

        match request {
            DialogRequest::OpenFile(req) => {
                kind = FileDialogKind::OpenFile;
                title = if req.title.is_empty() {
                    String::from("Open File")
                } else {
                    req.title.clone()
                };
                if let Some(dir) = req.initial_dir.as_ref().and_then(|d| PathBuf::from_str(d)) {
                    current_dir = dir;
                }
                confirm_label = req
                    .confirm_button_label
                    .clone()
                    .unwrap_or_else(|| String::from("Open"));
                show_preview = req.show_preview;
                multi_select_requested = req.allow_multiple;
                allowed_mime_types = req.allowed_mime_types.clone();
                allowed_extensions = req.allowed_extensions.clone();
            }
            DialogRequest::OpenFolder(req) => {
                kind = FileDialogKind::OpenFolder;
                title = if req.title.is_empty() {
                    String::from("Open Folder")
                } else {
                    req.title.clone()
                };
                if let Some(dir) = req.initial_dir.as_ref().and_then(|d| PathBuf::from_str(d)) {
                    current_dir = dir;
                }
                confirm_label = req
                    .confirm_button_label
                    .clone()
                    .unwrap_or_else(|| String::from("Choose Folder"));
            }
            DialogRequest::SaveFile(req) => {
                kind = FileDialogKind::SaveFile;
                title = if req.title.is_empty() {
                    String::from("Save File")
                } else {
                    req.title.clone()
                };
                if let Some(dir) = req.initial_dir.as_ref().and_then(|d| PathBuf::from_str(d)) {
                    current_dir = dir;
                }
                confirm_label = req
                    .confirm_button_label
                    .clone()
                    .unwrap_or_else(|| String::from("Save"));
                overwrite_confirm = req.overwrite_confirm;
                default_extension = req.default_extension.clone();
                allowed_extensions = req.allowed_extensions.clone();
                if let Some(name) = req.suggested_name.as_ref() {
                    input.set_text(name);
                }
                input.active = true;
            }
            _ => {}
        }

        let mut out = Self {
            kind,
            title,
            confirm_label,
            current_dir,
            selected: None,
            hover_row: None,
            scroll_offset: 0,
            rows: [FileRow::empty(); LIST_MAX],
            row_count: 0,
            status: String::new(),
            details: [String::new(), String::new(), String::new()],
            show_preview,
            input,
            input_active: matches!(kind, FileDialogKind::SaveFile),
            multi_select_requested,
            allowed_mime_types,
            allowed_extensions,
            default_extension,
            overwrite_confirm,
            pending_overwrite_confirm: false,
        };
        out.load_directory(current_dir);
        out
    }

    fn load_directory(&mut self, path: PathBuf) {
        let mut entries = [DirEntry::zeroed(); LIST_MAX];
        match libc::read_dir(path.as_str().as_bytes(), &mut entries) {
            Ok(count) => {
                self.row_count = count.min(LIST_MAX);
                entries[..self.row_count].sort_by(compare_entries);
                for (idx, entry) in entries[..self.row_count].iter().enumerate() {
                    self.rows[idx] = FileRow {
                        entry: *entry,
                        selectable: self.is_entry_selectable(*entry),
                    };
                }
                self.current_dir = path;
                self.selected = None;
                self.scroll_offset = 0;
                if self.multi_select_requested && self.kind == FileDialogKind::OpenFile {
                    self.status = String::from("Multiple selection is not supported yet");
                } else {
                    self.status.clear();
                }
                self.update_details();
            }
            Err(_) => {
                self.row_count = 0;
                self.selected = None;
                self.status = String::from("Unable to read directory");
                self.update_details();
            }
        }
    }

    fn navigate_up(&mut self) {
        if let Some(parent) = self.current_dir.parent() {
            self.load_directory(parent);
        }
    }

    fn row_at(&self, x: i32, y: i32, body: Rect) -> Option<usize> {
        if !body.contains(Point::new(x, y)) {
            return None;
        }
        let local_y = y - body.y - HEADER_H;
        if local_y < 0 {
            return None;
        }
        let row = self.scroll_offset + (local_y / ROW_H) as usize;
        (row < self.row_count).then_some(row)
    }

    fn visible_rows(&self, body: Rect) -> usize {
        ((body.h as i32 - HEADER_H - STATUS_H) / ROW_H).max(1) as usize
    }

    fn move_selection(&mut self, delta: isize, body: Rect) {
        if self.row_count == 0 {
            return;
        }
        let current = self.selected.unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, self.row_count.saturating_sub(1) as isize) as usize;
        self.selected = Some(next);
        let visible = self.visible_rows(body);
        if next < self.scroll_offset {
            self.scroll_offset = next;
        } else if next >= self.scroll_offset + visible {
            self.scroll_offset = next.saturating_sub(visible.saturating_sub(1));
        }
        self.pending_overwrite_confirm = false;
        self.update_details();
        self.sync_save_input_to_selection();
    }

    fn activate_selection(&mut self) -> Option<DialogResult> {
        let Some(index) = self.selected else {
            return self.default_action();
        };
        let row = self.rows[index];
        if row.entry.file_type == FT_DIR {
            if self.kind == FileDialogKind::OpenFolder {
                return Some(self.choose_folder(row));
            }
            if let Some(path) = self.current_dir.join(row.name_str()) {
                self.load_directory(path);
            }
            None
        } else {
            match self.kind {
                FileDialogKind::OpenFile => self.choose_file(row),
                FileDialogKind::OpenFolder => None,
                FileDialogKind::SaveFile => self.accept_save(),
            }
        }
    }

    fn default_action(&mut self) -> Option<DialogResult> {
        match self.kind {
            FileDialogKind::OpenFolder => Some(DialogResult::FolderSelected(String::from(
                self.current_dir.as_str(),
            ))),
            FileDialogKind::SaveFile => self.accept_save(),
            FileDialogKind::OpenFile => None,
        }
    }

    fn choose_file(&self, row: FileRow) -> Option<DialogResult> {
        if !row.selectable || row.entry.file_type != FT_FILE {
            return None;
        }
        let path = self.current_dir.join(row.name_str())?;
        Some(DialogResult::FileSelected(String::from(path.as_str())))
    }

    fn choose_folder(&self, row: FileRow) -> DialogResult {
        let path = self
            .current_dir
            .join(row.name_str())
            .unwrap_or(self.current_dir);
        DialogResult::FolderSelected(String::from(path.as_str()))
    }

    fn accept_save(&mut self) -> Option<DialogResult> {
        let mut name = String::from(self.input.value().trim());
        if name.is_empty() {
            self.status = String::from("Enter a file name");
            return None;
        }
        if name.contains('/') {
            self.status = String::from("File name cannot contain /");
            return None;
        }
        if let Some(default_ext) = self.default_extension.as_ref() {
            if !has_extension(name.as_bytes()) {
                if !default_ext.is_empty() {
                    if !default_ext.starts_with('.') {
                        name.push('.');
                    }
                    name.push_str(default_ext.trim_start_matches('.'));
                }
            }
        }
        if !self.allowed_extensions.is_empty()
            && !matches_extension(&name, &self.allowed_extensions)
        {
            self.status = String::from("File extension is not allowed");
            return None;
        }
        let full = self.current_dir.join(name.as_str())?;
        if self.overwrite_confirm && libc::stat(full.as_str().as_bytes()).is_ok() {
            if !self.pending_overwrite_confirm {
                self.pending_overwrite_confirm = true;
                self.status = String::from("Press Save again to overwrite");
                return None;
            }
        }
        Some(DialogResult::SavePathSelected(String::from(full.as_str())))
    }

    fn primary_result(&mut self) -> Option<DialogResult> {
        match self.kind {
            FileDialogKind::OpenFile => self.activate_selection(),
            FileDialogKind::OpenFolder => {
                if let Some(index) = self.selected {
                    let row = self.rows[index];
                    if row.entry.file_type == FT_DIR {
                        Some(self.choose_folder(row))
                    } else {
                        self.default_action()
                    }
                } else {
                    self.default_action()
                }
            }
            FileDialogKind::SaveFile => self.accept_save(),
        }
    }

    fn current_preview_lines(&self) -> [&str; INFO_LINES] {
        [
            self.details[0].as_str(),
            self.details[1].as_str(),
            self.details[2].as_str(),
        ]
    }

    fn update_details(&mut self) {
        self.details = [String::new(), String::new(), String::new()];
        if let Some(index) = self.selected {
            let row = self.rows[index];
            self.details[0] = String::from(row.name_str());
            if row.entry.file_type == FT_DIR {
                self.details[1] = String::from("Folder");
                self.details[2] = String::from("Double-click or Enter to open");
            } else {
                let mime = sun_open::mime_from_path(row.entry.name_bytes());
                self.details[1] = String::from(core::str::from_utf8(mime).unwrap_or("file"));
                self.details[2] = format_size(row.entry.size);
            }
        } else {
            self.details[0] = String::from(self.current_dir.as_str());
            self.details[1] = String::from(match self.kind {
                FileDialogKind::OpenFile => "Select a file to open",
                FileDialogKind::OpenFolder => "Choose the current folder or a subfolder",
                FileDialogKind::SaveFile => "Enter a name, then save",
            });
        }
    }

    fn sync_save_input_to_selection(&mut self) {
        if self.kind != FileDialogKind::SaveFile {
            return;
        }
        if let Some(index) = self.selected {
            let row = self.rows[index];
            if row.entry.file_type == FT_FILE {
                self.input.set_text(row.name_str());
            }
        }
    }

    fn is_entry_selectable(&self, entry: DirEntry) -> bool {
        if entry.file_type == FT_DIR {
            return true;
        }
        match self.kind {
            FileDialogKind::OpenFolder => false,
            FileDialogKind::OpenFile | FileDialogKind::SaveFile => {
                let name = entry.name_bytes();
                let mime = sun_open::mime_from_path(name);
                let mime_ok = self.allowed_mime_types.is_empty()
                    || self
                        .allowed_mime_types
                        .iter()
                        .any(|item| item.as_bytes() == mime);
                let ext_ok = self.allowed_extensions.is_empty()
                    || matches_extension(
                        core::str::from_utf8(name).unwrap_or(""),
                        &self.allowed_extensions,
                    );
                mime_ok && ext_ok
            }
        }
    }
}

enum DialogMode {
    Simple(SimpleDialogState),
    File(FileDialogState),
}

struct SimpleDialogState {
    request: DialogRequest,
    hover_primary: bool,
    hover_secondary: bool,
    focus_primary: bool,
    input: TextInput<'static, { sunlight_dialogs::MAX_TEXT_BYTES }>,
    shake_invalid: bool,
}

impl SimpleDialogState {
    fn new(request: DialogRequest) -> Self {
        let mut input = TextInput::new(Rect::new(PAD, 0, ALERT_W - (PAD as u32 * 2), TEXT_INPUT_H));
        let focus_primary = match &request {
            DialogRequest::Confirm(req) => {
                !matches!(req.default_button, DialogButton::Cancel | DialogButton::No)
            }
            _ => true,
        };
        if let DialogRequest::TextInput(TextInputRequest { default_value, .. }) = &request {
            input.set_text(default_value);
            input.active = true;
        }
        Self {
            request,
            hover_primary: false,
            hover_secondary: false,
            focus_primary,
            input,
            shake_invalid: false,
        }
    }

    fn buttons(&self) -> (&'static str, Option<&'static str>) {
        match &self.request {
            DialogRequest::Alert(_) => ("OK", None),
            DialogRequest::Confirm(req) => {
                let (primary, secondary) = confirm_labels(req.style);
                (primary, Some(secondary))
            }
            DialogRequest::TextInput(_) => ("OK", Some("Cancel")),
            _ => ("OK", None),
        }
    }
}

struct DialogApp {
    mode: DialogMode,
    result: Option<DialogResult>,
    height: u32,
    theme_icons: MimeIconTheme,
}

impl DialogApp {
    fn new(request: DialogRequest, height: u32) -> Self {
        let mode = match request {
            DialogRequest::Alert(_) | DialogRequest::Confirm(_) | DialogRequest::TextInput(_) => {
                DialogMode::Simple(SimpleDialogState::new(request))
            }
            DialogRequest::OpenFile(_)
            | DialogRequest::OpenFolder(_)
            | DialogRequest::SaveFile(_) => {
                DialogMode::File(FileDialogState::from_request(&request))
            }
            _ => DialogMode::Simple(SimpleDialogState::new(DialogRequest::Alert(
                sunlight_dialogs::AlertRequest {
                    common: sunlight_dialogs::DialogCommonOptions {
                        title: String::from("Unsupported"),
                        message: String::from("Dialog type not implemented"),
                        severity: sunlight_dialogs::DialogSeverity::Information,
                        silent: true,
                    },
                },
            ))),
        };
        Self {
            mode,
            result: None,
            height,
            theme_icons: MimeIconTheme::load(),
        }
    }

    fn primary_button_rect(&self) -> Rect {
        let w = if matches!(self.mode, DialogMode::File(_)) {
            FILE_W
        } else {
            ALERT_W
        };
        let h = self.height;
        Rect::new(
            w as i32 - PAD - BTN_W as i32 - 16,
            h as i32 - PAD - BTN_H as i32 - 10,
            BTN_W,
            BTN_H,
        )
    }

    fn secondary_button_rect(&self) -> Rect {
        let primary = self.primary_button_rect();
        Rect::new(primary.x - BTN_W as i32 - 10, primary.y, BTN_W, BTN_H)
    }

    fn simple_card_rect(&self) -> Rect {
        Rect::new(10, 10, ALERT_W - 20, self.height - 20)
    }

    fn file_card_rect(&self) -> Rect {
        Rect::new(10, 10, FILE_W - 20, self.height - 20)
    }
}

impl App for DialogApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        let width = if matches!(self.mode, DialogMode::File(_)) {
            FILE_W
        } else {
            ALERT_W
        };
        let simple_card = self.simple_card_rect();
        let file_card = self.file_card_rect();
        let primary_rect = self.primary_button_rect();
        let secondary_rect = self.secondary_button_rect();
        canvas.fill_rect(
            Rect::new(0, 0, width, self.height),
            Color::rgba(0x12, 0x12, 0x14, 0xF2),
        );
        match &mut self.mode {
            DialogMode::Simple(state) => draw_simple_dialog(
                canvas,
                theme,
                state,
                simple_card,
                primary_rect,
                secondary_rect,
            ),
            DialogMode::File(state) => draw_file_dialog(
                canvas,
                theme,
                &self.theme_icons,
                state,
                file_card,
                primary_rect,
                secondary_rect,
            ),
        }
    }

    fn update(&mut self, event: Event) -> bool {
        let file_card = self.file_card_rect();
        let primary_rect = self.primary_button_rect();
        let secondary_rect = self.secondary_button_rect();
        match &mut self.mode {
            DialogMode::Simple(state) => {
                update_simple_dialog(state, event, primary_rect, secondary_rect, &mut self.result)
            }
            DialogMode::File(state) => update_file_dialog(
                state,
                event,
                file_card,
                primary_rect,
                secondary_rect,
                &mut self.result,
            ),
        }
    }
}

fn present_dialog(request: DialogRequest) -> Result<DialogResult, DialogError> {
    let semantic_sound = request.system_sound();
    let (title, width, height) = match &request {
        DialogRequest::OpenFile(_) | DialogRequest::OpenFolder(_) | DialogRequest::SaveFile(_) => {
            ("Sunlight Dialog", FILE_W, FILE_H)
        }
        DialogRequest::TextInput(_) => ("Sunlight Dialog", ALERT_W, INPUT_H),
        _ => ("Sunlight Dialog", ALERT_W, ALERT_H),
    };
    let mut window = Window::connect(WindowConfig {
        width,
        height,
        title,
        decoration: WindowDecoration::HiddenOverlay,
    })
    .ok_or(DialogError::HostUnavailable)?;
    if let Some(sound) = semantic_sound {
        // Best effort: audio is supplemental and must never prevent rendering.
        let _ = AudioClient::new().play_system_sound(sound);
    }
    let flags = 1 | (1 << 5) | (90 << 6);
    window.configure_flags(flags);
    let mut app = DialogApp::new(request, height);
    window.run(&mut app);
    app.result.ok_or(DialogError::Internal)
}

fn draw_simple_dialog(
    canvas: &mut Canvas,
    theme: &Theme,
    state: &mut SimpleDialogState,
    card: Rect,
    primary_rect: Rect,
    secondary_rect: Rect,
) {
    let severity = match &state.request {
        DialogRequest::Alert(req) => req.common.severity,
        DialogRequest::Confirm(req) => req.common.severity,
        DialogRequest::TextInput(req) => req.common.severity,
        _ => DialogSeverity::Information,
    };
    let semantic_color = match severity {
        DialogSeverity::Information | DialogSeverity::Question => theme.accent,
        DialogSeverity::Success => theme.ok,
        DialogSeverity::Warning => theme.warn,
        DialogSeverity::Error | DialogSeverity::Critical => theme.danger,
    };
    canvas.fill_rounded_rect_with_border(card, RADIUS, theme.panel, semantic_color, 2);
    let (title, message) = match &state.request {
        DialogRequest::Alert(req) => (req.common.title.as_str(), req.common.message.as_str()),
        DialogRequest::Confirm(req) => (req.common.title.as_str(), req.common.message.as_str()),
        DialogRequest::TextInput(req) => (req.common.title.as_str(), req.common.message.as_str()),
        _ => ("", ""),
    };
    sf_draw(
        canvas,
        if title.is_empty() { " " } else { title },
        card.x + PAD,
        card.y + PAD,
        &TextStyle::new(FontRole::UiTitle, semantic_color),
    );
    let line_h = sf_lh(FontRole::UiRegular) as i32 + 2;
    let mut line_y = card.y + PAD + 30;
    for line in wrap_lines(message, card.w.saturating_sub((PAD as u32) * 2), 4).iter() {
        sf_draw(
            canvas,
            line.as_str(),
            card.x + PAD,
            line_y,
            &TextStyle::new(FontRole::UiRegular, theme.text),
        );
        line_y += line_h;
    }
    if matches!(state.request, DialogRequest::TextInput(_)) {
        state.input.rect = Rect::new(
            card.x + PAD,
            card.y + 118,
            card.w - (PAD as u32 * 2),
            TEXT_INPUT_H,
        );
        state.input.draw(canvas, theme);
        if state.shake_invalid {
            sf_draw(
                canvas,
                "Value required",
                state.input.rect.x + 4,
                state.input.rect.bottom() + 8,
                &TextStyle::new(FontRole::UiSmall, theme.warn),
            );
        }
    }
    let (primary_label, secondary_label) = state.buttons();
    let mut primary = Button::new(primary_rect, primary_label);
    primary.state = if state.hover_primary {
        ButtonState::Hovered
    } else {
        ButtonState::Normal
    };
    primary.draw(canvas, theme);
    if let Some(label) = secondary_label {
        let mut secondary = Button::secondary(secondary_rect, label);
        secondary.state = if state.hover_secondary {
            ButtonState::Hovered
        } else {
            ButtonState::Normal
        };
        secondary.draw(canvas, theme);
    }
}

fn update_simple_dialog(
    state: &mut SimpleDialogState,
    event: Event,
    primary_rect: Rect,
    secondary_rect: Rect,
    result: &mut Option<DialogResult>,
) -> bool {
    if matches!(state.request, DialogRequest::TextInput(_)) && state.input.context_menu_open() {
        return state.input.update(event);
    }
    match event {
        Event::MouseMove { x, y } => {
            if matches!(state.request, DialogRequest::TextInput(_)) {
                let _ = state.input.update(event);
            }
            let pt = Point::new(x, y);
            state.hover_primary = primary_rect.contains(pt);
            state.hover_secondary = secondary_rect.contains(pt);
            true
        }
        Event::Click { x, y } => {
            let pt = Point::new(x, y);
            if primary_rect.contains(pt) {
                if let Some(out) = simple_primary_result(state) {
                    *result = Some(out);
                    request_close();
                }
                return true;
            }
            if state.buttons().1.is_some() && secondary_rect.contains(pt) {
                *result = Some(simple_secondary_result(state));
                request_close();
                return true;
            }
            if matches!(state.request, DialogRequest::TextInput(_)) {
                return state.input.update(event);
            }
            false
        }
        Event::Key('\n')
        | Event::KeyPress {
            keycode: KEY_ENTER,
            pressed: true,
            ..
        } => {
            let out = if state.focus_primary || state.buttons().1.is_none() {
                simple_primary_result(state)
            } else {
                Some(simple_secondary_result(state))
            };
            if let Some(out) = out {
                *result = Some(out);
                request_close();
            }
            true
        }
        Event::KeyPress {
            keycode: KEY_ESC,
            pressed: true,
            ..
        } => {
            *result = Some(simple_dismiss_result(state));
            request_close();
            true
        }
        Event::KeyPress {
            keycode: KEY_TAB,
            pressed: true,
            ..
        } => {
            if state.buttons().1.is_some() {
                state.focus_primary = !state.focus_primary;
            }
            true
        }
        _ => {
            if matches!(state.request, DialogRequest::TextInput(_)) {
                let changed = state.input.update(event);
                if changed {
                    state.shake_invalid = false;
                }
                changed
            } else {
                false
            }
        }
    }
}

fn simple_primary_result(state: &mut SimpleDialogState) -> Option<DialogResult> {
    Some(match &state.request {
        DialogRequest::Alert(_) => DialogResult::Ok,
        DialogRequest::Confirm(req) => match req.style {
            ConfirmStyle::OkCancel => DialogResult::Ok,
            ConfirmStyle::YesNo => DialogResult::Yes,
        },
        DialogRequest::TextInput(req) => {
            let value = state.input.value();
            if !req.allow_empty && value.is_empty() {
                state.shake_invalid = true;
                return None;
            }
            DialogResult::TextSubmitted(String::from(value))
        }
        _ => DialogResult::Dismissed,
    })
}

fn simple_secondary_result(state: &SimpleDialogState) -> DialogResult {
    match &state.request {
        DialogRequest::Confirm(req) => match req.style {
            ConfirmStyle::OkCancel => DialogResult::Cancel,
            ConfirmStyle::YesNo => DialogResult::No,
        },
        DialogRequest::TextInput(_) => DialogResult::Cancel,
        _ => DialogResult::Dismissed,
    }
}

fn simple_dismiss_result(state: &SimpleDialogState) -> DialogResult {
    match &state.request {
        DialogRequest::Alert(_) => DialogResult::Ok,
        DialogRequest::Confirm(req) => match req.style {
            ConfirmStyle::OkCancel => DialogResult::Cancel,
            ConfirmStyle::YesNo => DialogResult::No,
        },
        DialogRequest::TextInput(_) => DialogResult::Cancel,
        _ => DialogResult::Dismissed,
    }
}

fn draw_file_dialog(
    canvas: &mut Canvas,
    theme: &Theme,
    icons: &MimeIconTheme,
    state: &mut FileDialogState,
    card: Rect,
    primary_rect: Rect,
    secondary_rect: Rect,
) {
    canvas.fill_rounded_rect_with_border(card, RADIUS, theme.panel, theme.accent, 2);
    sf_draw(
        canvas,
        state.title.as_str(),
        card.x + PAD,
        card.y + PAD,
        &TextStyle::new(FontRole::UiTitle, theme.accent),
    );
    let path_rect = Rect::new(card.x + PAD, card.y + 40, card.w - (PAD as u32 * 2), 22);
    canvas.fill_rect(path_rect, theme.panel_alt);
    canvas.draw_rect(path_rect, theme.border);
    sf_draw(
        canvas,
        fit_text_prefix(
            state.current_dir.as_str(),
            FontRole::UiSmall,
            path_rect.w.saturating_sub(12),
        ),
        path_rect.x + 6,
        path_rect.y + 5,
        &TextStyle::new(FontRole::UiSmall, theme.text),
    );

    let body = file_body_rect(card, matches!(state.kind, FileDialogKind::SaveFile));
    canvas.fill_rect(body, theme.panel_alt);
    canvas.draw_rect(body, theme.border);
    draw_file_header(canvas, theme, body);

    let visible = state.visible_rows(body);
    for local_idx in 0..visible {
        let row_idx = state.scroll_offset + local_idx;
        if row_idx >= state.row_count {
            break;
        }
        let row = state.rows[row_idx];
        let row_rect = Rect::new(
            body.x + 1,
            body.y + HEADER_H + local_idx as i32 * ROW_H,
            body.w.saturating_sub(2),
            ROW_H as u32,
        );
        let hovered = state.hover_row == Some(row_idx);
        let selected = state.selected == Some(row_idx);
        let bg = if selected {
            theme.accent.darken(175)
        } else if hovered {
            theme.panel.lighten(10)
        } else if row_idx % 2 == 0 {
            theme.panel
        } else {
            theme.panel_alt
        };
        canvas.fill_rect(row_rect, bg);
        let icon_rect = Rect::new(row_rect.x + 6, row_rect.y + 5, 22, 22);
        if let Some(icon) = icons.icon_for_entry(&row.entry) {
            canvas.draw_tga_icon(&icon, icon_rect);
        } else {
            canvas.draw_ui_symbol_centered(
                icon_rect,
                if row.entry.file_type == FT_DIR {
                    UiSymbol::Folder
                } else {
                    UiSymbol::File
                },
                theme.accent,
            );
        }
        let text_color = if row.selectable {
            theme.text
        } else {
            theme.text_dim
        };
        let name_w = body.w.saturating_sub(160);
        sf_draw(
            canvas,
            fit_text_prefix(row.name_str(), FontRole::UiRegular, name_w),
            row_rect.x + 34,
            row_rect.y + 9,
            &TextStyle::new(FontRole::UiRegular, text_color),
        );
        let type_label = if row.entry.file_type == FT_DIR {
            "Folder"
        } else {
            mime_label(row.entry.name_bytes())
        };
        sf_draw(
            canvas,
            fit_text_prefix(type_label, FontRole::UiSmall, 92),
            row_rect.right() - 116,
            row_rect.y + 10,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        canvas.hbar(
            row_rect.x,
            row_rect.bottom() - 1,
            row_rect.w,
            1,
            theme.border,
        );
    }

    let preview = file_preview_rect(card, matches!(state.kind, FileDialogKind::SaveFile));
    if state.show_preview || state.kind != FileDialogKind::OpenFile {
        canvas.fill_rect(preview, theme.panel_alt);
        canvas.draw_rect(preview, theme.border);
        let lines = state.current_preview_lines();
        for (idx, line) in lines.iter().enumerate() {
            sf_draw(
                canvas,
                fit_text_prefix(line, FontRole::UiSmall, preview.w.saturating_sub(16)),
                preview.x + 8,
                preview.y + 10 + idx as i32 * 18,
                &TextStyle::new(
                    FontRole::UiSmall,
                    if idx == 0 { theme.text } else { theme.text_dim },
                ),
            );
        }
    }

    if matches!(state.kind, FileDialogKind::SaveFile) {
        let input_rect = save_name_rect(card);
        sf_draw(
            canvas,
            "Name",
            input_rect.x,
            input_rect.y - 16,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        state.input.rect = input_rect;
        state.input.draw(canvas, theme);
    }

    let up_rect = up_button_rect(card);
    let up = Button::secondary(up_rect, "Up");
    up.draw(canvas, theme);
    let cancel = Button::secondary(secondary_rect, "Cancel");
    cancel.draw(canvas, theme);
    let mut primary = Button::new(primary_rect, state.confirm_label.as_str());
    if matches!(state.kind, FileDialogKind::SaveFile) && state.input.value().trim().is_empty() {
        primary.state = ButtonState::Disabled;
    }
    primary.draw(canvas, theme);

    if !state.status.is_empty() {
        sf_draw(
            canvas,
            fit_text_prefix(
                state.status.as_str(),
                FontRole::UiSmall,
                card.w.saturating_sub(220),
            ),
            card.x + PAD,
            card.bottom() - PAD - 6,
            &TextStyle::new(
                FontRole::UiSmall,
                if state.pending_overwrite_confirm {
                    theme.warn
                } else {
                    theme.text_dim
                },
            ),
        );
    }
}

fn update_file_dialog(
    state: &mut FileDialogState,
    event: Event,
    card: Rect,
    primary_rect: Rect,
    secondary_rect: Rect,
    result: &mut Option<DialogResult>,
) -> bool {
    let body = file_body_rect(card, matches!(state.kind, FileDialogKind::SaveFile));
    if matches!(state.kind, FileDialogKind::SaveFile) && state.input.context_menu_open() {
        return state.input.update(event);
    }
    match event {
        Event::MouseMove { x, y } => {
            if matches!(state.kind, FileDialogKind::SaveFile) {
                let _ = state.input.update(event);
            }
            state.hover_row = state.row_at(x, y, body);
            true
        }
        Event::Click { x, y } => {
            let pt = Point::new(x, y);
            if secondary_rect.contains(pt) {
                *result = Some(DialogResult::Cancelled);
                request_close();
                return true;
            }
            if primary_rect.contains(pt) {
                if let Some(out) = state.primary_result() {
                    *result = Some(out);
                    request_close();
                }
                return true;
            }
            if up_button_rect(card).contains(pt) {
                state.navigate_up();
                return true;
            }
            if matches!(state.kind, FileDialogKind::SaveFile) && state.input.rect.contains(pt) {
                state.input_active = true;
                state.input.update(event);
                return true;
            }
            if let Some(row_idx) = state.row_at(x, y, body) {
                state.selected = Some(row_idx);
                state.pending_overwrite_confirm = false;
                state.update_details();
                state.sync_save_input_to_selection();
                if let Some(row) = state.rows.get(row_idx).copied() {
                    if row.entry.file_type == FT_DIR
                        && matches!(state.kind, FileDialogKind::OpenFolder)
                        && state.kind != FileDialogKind::SaveFile
                    {
                        // single click only selects
                    }
                }
                return true;
            }
            false
        }
        Event::MouseDown { x, y, button } => {
            if matches!(state.kind, FileDialogKind::SaveFile)
                && state.input.update(Event::MouseDown { x, y, button })
            {
                return true;
            }
            if button == 0 {
                if let Some(row_idx) = state.row_at(x, y, body) {
                    state.selected = Some(row_idx);
                    state.pending_overwrite_confirm = false;
                    state.update_details();
                    state.sync_save_input_to_selection();
                    return true;
                }
            }
            false
        }
        Event::Key('\n')
        | Event::KeyPress {
            keycode: KEY_ENTER,
            pressed: true,
            ..
        } => {
            if let Some(out) = state.activate_selection() {
                *result = Some(out);
                request_close();
            }
            true
        }
        Event::KeyPress {
            keycode: KEY_ESC,
            pressed: true,
            ..
        } => {
            *result = Some(DialogResult::Cancelled);
            request_close();
            true
        }
        Event::KeyPress {
            keycode: KEY_UP,
            pressed: true,
            ..
        } => {
            state.move_selection(-1, body);
            true
        }
        Event::KeyPress {
            keycode: KEY_DOWN,
            pressed: true,
            ..
        } => {
            state.move_selection(1, body);
            true
        }
        Event::KeyPress {
            keycode: KEY_LEFT,
            pressed: true,
            ..
        } => {
            state.navigate_up();
            true
        }
        Event::Key(KEY_BACKSPACE) if matches!(state.kind, FileDialogKind::SaveFile) => {
            state.input_active = true;
            state.pending_overwrite_confirm = false;
            state.status.clear();
            state.input.update(event)
        }
        _ if matches!(state.kind, FileDialogKind::SaveFile) => {
            let changed = state.input.update(event);
            if changed {
                state.pending_overwrite_confirm = false;
                state.status.clear();
            }
            changed
        }
        _ => false,
    }
}

fn file_body_rect(card: Rect, save_mode: bool) -> Rect {
    let bottom_reserved = if save_mode { 150 } else { 108 };
    Rect::new(
        card.x + PAD,
        card.y + 70,
        card.w - (PAD as u32 * 2),
        card.h.saturating_sub(bottom_reserved as u32),
    )
}

fn file_preview_rect(card: Rect, save_mode: bool) -> Rect {
    let body = file_body_rect(card, save_mode);
    Rect::new(body.x, body.bottom() + 10, body.w, 62)
}

fn save_name_rect(card: Rect) -> Rect {
    let preview = file_preview_rect(card, true);
    Rect::new(preview.x, preview.bottom() + 30, preview.w, TEXT_INPUT_H)
}

fn up_button_rect(card: Rect) -> Rect {
    Rect::new(card.right() - PAD - 64, card.y + 36, 64, 24)
}

fn draw_file_header(canvas: &mut Canvas, theme: &Theme, body: Rect) {
    canvas.fill_rect(
        Rect::new(body.x, body.y, body.w, HEADER_H as u32),
        theme.panel,
    );
    sf_draw(
        canvas,
        "Name",
        body.x + 36,
        body.y + 10,
        &TextStyle::new(FontRole::UiSmall, theme.text_dim),
    );
    sf_draw(
        canvas,
        "Type",
        body.right() - 116,
        body.y + 10,
        &TextStyle::new(FontRole::UiSmall, theme.text_dim),
    );
    canvas.hbar(body.x, body.y + HEADER_H - 1, body.w, 1, theme.border);
}

fn take_request_page(msg: &IpcMsg) -> Result<Vec<u8>, DialogError> {
    let len = msg.words[0] as usize;
    let token = msg.caps[0];
    if len == 0 || len > SHM_PAGE || token == CapabilityToken::INVALID {
        return Err(DialogError::BadRequest);
    }
    let ptr = shm_map(token).map_err(|_| DialogError::BadRequest)?;
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec();
    let _ = shm_free(token);
    Ok(bytes)
}

fn reply_with_bytes(label: u64, value0: u64, bytes: &[u8]) -> Result<IpcMsg, DialogError> {
    if bytes.len() > SHM_PAGE {
        return Err(DialogError::TooLarge);
    }
    let (ptr, token) = shm_alloc().map_err(|_| DialogError::Internal)?;
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
    }
    Ok(IpcMsg::with_label(label)
        .word(0, value0)
        .word(1, bytes.len() as u64)
        .with_cap(0, token))
}

struct DialogServer {
    ep: sunlight_ipc::EndpointId,
}

impl DialogServer {
    fn new() -> Self {
        let ep = endpoint_create();
        nameserver_register("dialogd", ep);
        Self { ep }
    }

    fn run(&mut self) -> ! {
        loop {
            let Some(msg) = ipc_recv_timeout(self.ep, SERVICE_POLL_MS) else {
                process_yield();
                continue;
            };
            let reply = self.handle_message(msg);
            ipc_reply(reply);
        }
    }

    fn handle_message(&mut self, msg: IpcMsg) -> IpcMsg {
        match msg.label {
            DialogMsg::SHOW_DIALOG => {
                let body = match take_request_page(&msg) {
                    Ok(body) => body,
                    Err(err) => return IpcMsg::with_label(DialogMsg::ERROR).word(0, err.code()),
                };
                let request = match decode_request(&body).and_then(|req| {
                    validate_request(&req)?;
                    Ok(req)
                }) {
                    Ok(req) => req,
                    Err(err) => return IpcMsg::with_label(DialogMsg::ERROR).word(0, err.code()),
                };
                match present_dialog(request) {
                    Ok(result) => {
                        match reply_with_bytes(DialogMsg::REPLY, 0, &encode_result(&result)) {
                            Ok(reply) => reply,
                            Err(err) => IpcMsg::with_label(DialogMsg::ERROR).word(0, err.code()),
                        }
                    }
                    Err(err) => IpcMsg::with_label(DialogMsg::ERROR).word(0, err.code()),
                }
            }
            _ => IpcMsg::with_label(DialogMsg::ERROR).word(0, DialogError::BadRequest.code()),
        }
    }
}

fn detect_home_path() -> PathBuf {
    if let Some(home) = env::getenv(b"HOME") {
        if let Some(path) = PathBuf::from_str(home) {
            return path;
        }
    }
    PathBuf::from_str("/root").unwrap_or_else(PathBuf::root)
}

fn compare_entries(a: &DirEntry, b: &DirEntry) -> Ordering {
    let a_dir = a.file_type == FT_DIR;
    let b_dir = b.file_type == FT_DIR;
    match (a_dir, b_dir) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => cmp_ascii_ci(a.name_bytes(), b.name_bytes()),
    }
}

fn cmp_ascii_ci(a: &[u8], b: &[u8]) -> Ordering {
    let len = a.len().min(b.len());
    for i in 0..len {
        match a[i].to_ascii_lowercase().cmp(&b[i].to_ascii_lowercase()) {
            Ordering::Equal => {}
            ord => return ord,
        }
    }
    a.len().cmp(&b.len())
}

fn fit_text_prefix<'a>(text: &'a str, role: FontRole, max_w: u32) -> &'a str {
    if sf_measure(text, role).w <= max_w {
        return text;
    }
    let bytes = text.as_bytes();
    let mut best = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        i += 1;
        while i < bytes.len() && !text.is_char_boundary(i) {
            i += 1;
        }
        if sf_measure(&text[..i], role).w <= max_w {
            best = i;
        } else {
            break;
        }
    }
    &text[..best]
}

fn wrap_lines(text: &str, max_w: u32, max_lines: usize) -> heapless::Vec<String, 6> {
    let mut out = heapless::Vec::<String, 6>::new();
    if text.is_empty() {
        let _ = out.push(String::new());
        return out;
    }
    let mut current = String::new();
    for word in text.split_whitespace() {
        let next = if current.is_empty() {
            String::from(word)
        } else {
            let mut tmp = current.clone();
            tmp.push(' ');
            tmp.push_str(word);
            tmp
        };
        if sf_measure(next.as_str(), FontRole::UiRegular).w <= max_w && next.len() <= 256 {
            current = next;
            continue;
        }
        if !current.is_empty() {
            let _ = out.push(current.clone());
            if out.len() >= max_lines {
                return out;
            }
        }
        current = truncate_to_width(word, max_w);
    }
    if out.len() < max_lines {
        let _ = out.push(current);
    }
    out
}

fn truncate_to_width(text: &str, max_w: u32) -> String {
    if sf_measure(text, FontRole::UiRegular).w <= max_w {
        return String::from(text);
    }
    let mut cut = String::new();
    for ch in text.chars() {
        let mut next = cut.clone();
        next.push(ch);
        let mut dotted = next.clone();
        dotted.push_str("..");
        if sf_measure(dotted.as_str(), FontRole::UiRegular).w > max_w {
            break;
        }
        cut = next;
    }
    cut.push_str("..");
    cut
}

fn mime_label(name: &[u8]) -> &'static str {
    let mime = sun_open::mime_from_path(name);
    if mime == b"application/json" {
        "JSON"
    } else if mime == b"text/plain" {
        "Text"
    } else if mime == b"text/markdown" {
        "Markdown"
    } else if mime == b"text/rust" {
        "Rust"
    } else if mime_icon::is_image_mime(mime) {
        "Image"
    } else if mime == b"application/x-executable" {
        "Executable"
    } else {
        "File"
    }
}

fn format_size(size: u64) -> String {
    if size < 1024 {
        let mut out = String::new();
        append_u64(&mut out, size);
        out.push_str(" B");
        return out;
    }
    if size < 1024 * 1024 {
        let mut out = String::new();
        append_u64(&mut out, size / 1024);
        out.push_str(" KiB");
        return out;
    }
    let mut out = String::new();
    append_u64(&mut out, size / (1024 * 1024));
    out.push_str(" MiB");
    out
}

fn append_u64(out: &mut String, mut value: u64) {
    let mut buf = [0u8; 20];
    let mut idx = buf.len();
    if value == 0 {
        out.push('0');
        return;
    }
    while value > 0 {
        idx -= 1;
        buf[idx] = b'0' + (value % 10) as u8;
        value /= 10;
    }
    out.push_str(core::str::from_utf8(&buf[idx..]).unwrap_or(""));
}

fn has_extension(name: &[u8]) -> bool {
    name.iter().rposition(|&b| b == b'.').is_some()
}

fn matches_extension(name: &str, allowed: &[String]) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("");
    if ext == name {
        return false;
    }
    allowed.iter().any(|item| {
        let candidate = item.trim_start_matches('.');
        candidate.eq_ignore_ascii_case(ext)
    })
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[DIALOGD] starting\n");
    let mut server = DialogServer::new();
    debug_log("[DIALOGD] registered\n");
    server.run();
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    debug_log("[DIALOGD] PANIC\n");
    ProcessExit::exit(101);
}
