#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![cfg_attr(test, allow(dead_code, unused_imports))]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::GlobalAlloc;

use sun_font::{
    draw_text, draw_text_vcenter, line_height, measure_text, FontRole, TextStyle, VecFont,
};
use sunlight_dialogs::{
    decode_result, encode_request, ConfirmRequest, ConfirmStyle,
    DialogButton as DialogChoiceButton, DialogCommonOptions, DialogError, DialogMsg, DialogRequest,
    DialogResult, OpenFileRequest, SaveFileRequest,
};
use sunlight_edit::args::extract_first_real_file_path;
use sunlight_edit::text_buffer::{TextBuffer, TextRange};
use sunlight_ipc::{
    debug_log, ipc_call,
    launch_trace::{self, LaunchSource, LaunchTrace},
    monotonic_millis, nameserver_lookup, nameserver_lookup_timeout, process_yield, shm_alloc,
    shm_free, shm_map, CapabilityToken, IpcMsg, ProcessExit, SHM_PAGE,
};
use sunlight_libc::{self as libc, crt0};
use sunlight_ui::image::TgaImage;
use sunlight_ui::widgets::button::ButtonState;
use sunlight_ui::widgets::{
    StatusBar, TextCommand, TextEditor, TextEditorResponse, TextEditorState, TextInput,
};
use sunlight_ui::{
    request_close, App, AxisSizing, Canvas, Color, Column, Event, LayoutBox, LayoutInvalidation,
    Point, Rect, Size, Sizing, Theme, Window, WindowConfig, WindowDecoration, WindowEvent,
};

const WIN_W: u32 = 900;
const WIN_H: u32 = 640;
const HEADER_H: u32 = 34;
const TOOLBAR_H: u32 = 38;
const STATUS_H: u32 = 22;
const GUTTER_W: u32 = 56;
const PAD: i32 = 8;
const PATH_LEN: usize = 256;
const MSG_LEN: usize = 96;
const MAX_FILE_BYTES: usize = 512 * 1024;
const MAX_ARGC: usize = 8;
const UNTITLED_DISPLAY: &str = "Untitled";
const FIND_QUERY_MAX: usize = 128;
const FIND_REPLACE_MAX: usize = 128;
const TOOLBAR_ICON: u32 = 16;
const TOOLBAR_BTN_W: u32 = 40;
const TOOLBAR_GAP: i32 = 4;
const MENU_W: u32 = 184;
const MENU_ITEM_H: u32 = 24;
const FIND_PANEL_H: u32 = 76;
const CLIP_SOURCE_APP: &[u8] = b"sunlight-edit";

static F_MONO: VecFont = VecFont(FontRole::MonoRegular);

const KEY_ESC: u8 = 0x01;
const KEY_A: u8 = 0x1E;
const KEY_C: u8 = 0x2E;
const KEY_F: u8 = 0x21;
const KEY_H: u8 = 0x23;
const KEY_O: u8 = 0x18;
const KEY_V: u8 = 0x2F;
const KEY_X: u8 = 0x2D;
const KEY_Y: u8 = 0x15;
const KEY_Z: u8 = 0x2C;
const KEY_LEFT: u8 = 0x4B;
const KEY_RIGHT: u8 = 0x4D;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_HOME: u8 = 0x47;
const KEY_END: u8 = 0x4F;
const KEY_DELETE: u8 = 0x53;
const KEY_S: u8 = 0x1F;
const KEY_ENTER: u8 = 0x1C;

const DIALOG_W: u32 = 520;
const DIALOG_BTN_W: u32 = 88;
const DIALOG_BTN_H: u32 = 28;
const DIALOG_BTN_GAP: u32 = 10;
const DIALOG_PAD: i32 = 16;

// Icons now generated at build time from the Material Icons font (see build.rs).
// This replaces the checked-in symbolic TGAs with minitype-rasterised glyphs
// (smaller, consistent, lower RAM). All are white+alpha so they tint naturally.
static ICON_NEW_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_new.tga"));
static ICON_OPEN_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_open.tga"));
static ICON_SAVE_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_save.tga"));
static ICON_SAVE_AS_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_save_as.tga"));
static ICON_FIND_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_find.tga"));
static ICON_REPLACE_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_replace.tga"));
static ICON_CUT_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_cut.tga"));
static ICON_COPY_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_copy.tga"));
static ICON_PASTE_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_paste.tga"));
static ICON_SELECT_ALL_TGA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/icon_select_all.tga"));
static ICON_NEXT_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_next.tga"));
static ICON_PREV_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_prev.tga"));
// Hamburger replaces the text "Menu" label on the toolbar.
static ICON_MENU_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_menu.tga"));

#[cfg(not(test))]
struct BumpAllocator;
#[cfg(not(test))]
unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 2 * 1024 * 1024] = [0; 2 * 1024 * 1024];
        static mut NEXT: usize = 0;
        let aligned = (NEXT + layout.align() - 1) & !(layout.align() - 1);
        let end = aligned + layout.size();
        if end > HEAP.len() {
            return core::ptr::null_mut();
        }
        NEXT = end;
        HEAP.as_mut_ptr().add(aligned)
    }
    unsafe fn dealloc(&self, _: *mut u8, _: core::alloc::Layout) {}
}

#[cfg(not(test))]
#[global_allocator]
static ALLOC: BumpAllocator = BumpAllocator;

#[cfg(not(test))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[EDIT] panic\n");
    loop {
        process_yield();
    }
}

#[derive(Clone, Copy)]
struct PathBuf {
    buf: [u8; PATH_LEN],
    len: usize,
}

impl PathBuf {
    const fn empty() -> Self {
        Self {
            buf: [0; PATH_LEN],
            len: 0,
        }
    }

    fn from_str(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.is_empty() || bytes.len() >= PATH_LEN {
            return None;
        }
        let mut out = Self::empty();
        out.buf[..bytes.len()].copy_from_slice(bytes);
        out.len = bytes.len();
        Some(out)
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    fn file_name(&self) -> &str {
        let path = self.as_str();
        path.rsplit('/').next().unwrap_or(path)
    }
}

#[derive(Clone, Copy)]
struct TextSlot {
    buf: [u8; MSG_LEN],
    len: usize,
}

impl TextSlot {
    const fn empty() -> Self {
        Self {
            buf: [0; MSG_LEN],
            len: 0,
        }
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    fn set(&mut self, text: &str) {
        let bytes = text.as_bytes();
        self.len = bytes.len().min(MSG_LEN);
        self.buf[..self.len].copy_from_slice(&bytes[..self.len]);
    }

    fn as_str(&self) -> &str {
        if self.len == 0 {
            return "";
        }
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActiveDialog {
    None,
    SaveBeforeClose,
    SaveBeforeCloseTemporary,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DialogAction {
    Save,
    Discard,
    Cancel,
}

#[derive(Clone, Copy)]
struct DialogButton {
    action: DialogAction,
    rect: Rect,
    label: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FocusTarget {
    Editor,
    Find,
    Replace,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditorAction {
    New,
    Open,
    Save,
    SaveAs,
    Find,
    Replace,
    FindNext,
    FindPrev,
    ReplaceCurrent,
    ReplaceAll,
    SelectAll,
    Cut,
    Copy,
    Paste,
    About,
}

#[derive(Clone, Copy)]
struct MenuItemSpec {
    action: EditorAction,
    label: &'static str,
    icon: Option<&'static [u8]>,
}

#[derive(Clone, Copy)]
struct MenuItem {
    spec: MenuItemSpec,
    rect: Rect,
    enabled: bool,
}

#[derive(Clone, Copy)]
struct PopupMenu {
    rect: Rect,
    items: [MenuItem; 12],
    count: usize,
}

#[derive(Clone, Copy)]
struct ToolbarButtonSpec {
    action: EditorAction,
    label: &'static str,
}

#[derive(Clone, Copy)]
struct ToolbarButton {
    spec: ToolbarButtonSpec,
    rect: Rect,
}

#[derive(Clone, Copy)]
struct MatchHighlight {
    range: TextRange,
    current: bool,
}

struct FindState {
    visible: bool,
    replace_visible: bool,
    focus: FocusTarget,
    query: TextInput<'static, FIND_QUERY_MAX>,
    replace: TextInput<'static, FIND_REPLACE_MAX>,
    matches: Vec<TextRange>,
    current_match: Option<usize>,
    doc_revision: u64,
    query_revision: u64,
}

impl FindState {
    fn new() -> Self {
        let mut query = TextInput::new(Rect::new(0, 0, 0, 0));
        let replace = TextInput::new(Rect::new(0, 0, 0, 0));
        query.active = false;
        Self {
            visible: false,
            replace_visible: false,
            focus: FocusTarget::Editor,
            query,
            replace,
            matches: Vec::new(),
            current_match: None,
            doc_revision: 0,
            query_revision: 0,
        }
    }
}

struct EditorIcons {
    open: Option<TgaImage>,
    save: Option<TgaImage>,
    save_as: Option<TgaImage>,
    find: Option<TgaImage>,
    replace: Option<TgaImage>,
    cut: Option<TgaImage>,
    copy: Option<TgaImage>,
    paste: Option<TgaImage>,
    select_all: Option<TgaImage>,
    new_doc: Option<TgaImage>,
    next: Option<TgaImage>,
    prev: Option<TgaImage>,
    // Material hamburger for the toolbar menu button (was text "Menu").
    menu: Option<TgaImage>,
}

impl EditorIcons {
    fn load() -> Self {
        Self {
            open: TgaImage::parse(ICON_OPEN_TGA).ok(),
            save: TgaImage::parse(ICON_SAVE_TGA).ok(),
            save_as: TgaImage::parse(ICON_SAVE_AS_TGA).ok(),
            find: TgaImage::parse(ICON_FIND_TGA).ok(),
            replace: TgaImage::parse(ICON_REPLACE_TGA).ok(),
            cut: TgaImage::parse(ICON_CUT_TGA).ok(),
            copy: TgaImage::parse(ICON_COPY_TGA).ok(),
            paste: TgaImage::parse(ICON_PASTE_TGA).ok(),
            select_all: TgaImage::parse(ICON_SELECT_ALL_TGA).ok(),
            new_doc: TgaImage::parse(ICON_NEW_TGA).ok(),
            next: TgaImage::parse(ICON_NEXT_TGA).ok(),
            prev: TgaImage::parse(ICON_PREV_TGA).ok(),
            menu: TgaImage::parse(ICON_MENU_TGA).ok(),
        }
    }

    fn icon_for(&self, action: EditorAction) -> Option<&TgaImage> {
        match action {
            EditorAction::New => self.new_doc.as_ref(),
            EditorAction::Open => self.open.as_ref(),
            EditorAction::Save => self.save.as_ref(),
            EditorAction::SaveAs => self.save_as.as_ref(),
            EditorAction::Find => self.find.as_ref(),
            EditorAction::Replace => self.replace.as_ref(),
            EditorAction::FindNext => self.next.as_ref(),
            EditorAction::FindPrev => self.prev.as_ref(),
            EditorAction::ReplaceCurrent => self.replace.as_ref(),
            EditorAction::ReplaceAll => self.replace.as_ref(),
            EditorAction::SelectAll => self.select_all.as_ref(),
            EditorAction::Cut => self.cut.as_ref(),
            EditorAction::Copy => self.copy.as_ref(),
            EditorAction::Paste => self.paste.as_ref(),
            // About action on the toolbar is the hamburger / menu button.
            EditorAction::About => self.menu.as_ref(),
        }
    }
}

struct EditApp {
    buffer: TextBuffer,
    backing_path: Option<PathBuf>,
    user_path: Option<PathBuf>,
    is_temporary: bool,
    startup_error: TextSlot,
    status_left: TextSlot,
    status_center: TextSlot,
    status_right: TextSlot,
    header_title: TextSlot,
    scroll_line: usize,
    caret_visible: bool,
    caret_ticks: u8,
    status_msg_ticks: u16,
    toolbar_hover: Option<usize>,
    toolbar_pressed: Option<usize>,
    pending_user_path: Option<PathBuf>,
    document_ready: bool,
    active_dialog: ActiveDialog,
    dialog_buttons: [DialogButton; 3],
    dialog_button_count: usize,
    selection: TextEditorState,
    menu: Option<PopupMenu>,
    find: FindState,
    icons: EditorIcons,
    focus: FocusTarget,
    document_revision: u64,
    client_bounds: Rect,
    layout_invalidation: LayoutInvalidation,
    layout: EditLayout,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct EditLayout {
    root: Rect,
    header: Rect,
    toolbar: Rect,
    find: Rect,
    editor: Rect,
    status: Rect,
}

impl EditApp {
    fn new(pending_user_path: Option<PathBuf>) -> Self {
        let mut app = Self {
            buffer: TextBuffer::new(),
            backing_path: None,
            user_path: None,
            is_temporary: false,
            startup_error: TextSlot::empty(),
            status_left: TextSlot::empty(),
            status_center: TextSlot::empty(),
            status_right: TextSlot::empty(),
            header_title: TextSlot::empty(),
            scroll_line: 0,
            caret_visible: true,
            caret_ticks: 0,
            status_msg_ticks: 0,
            toolbar_hover: None,
            toolbar_pressed: None,
            pending_user_path,
            document_ready: false,
            active_dialog: ActiveDialog::None,
            dialog_buttons: [DialogButton {
                action: DialogAction::Cancel,
                rect: Rect::new(0, 0, 0, 0),
                label: "",
            }; 3],
            dialog_button_count: 0,
            selection: TextEditorState::new(),
            menu: None,
            find: FindState::new(),
            icons: EditorIcons::load(),
            focus: FocusTarget::Editor,
            document_revision: 0,
            client_bounds: Rect::new(0, 0, WIN_W, WIN_H),
            layout_invalidation: LayoutInvalidation::new(),
            layout: EditLayout::default(),
        };
        app.refresh_header_title();
        app.refresh_status_right();
        app.refresh_status_left();
        let _ = app.ensure_layout();
        app.layout_find_panel();
        app
    }

    fn compute_layout(root: Rect, find_visible: bool) -> EditLayout {
        let fill = Sizing::new(AxisSizing::Fill, AxisSizing::Fill);
        let fixed_height = |height| {
            LayoutBox::new(Rect::new(0, 0, 0, height))
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fixed(height)))
        };
        let find_height = if find_visible { FIND_PANEL_H } else { 0 };
        let mut children = [
            fixed_height(HEADER_H),
            fixed_height(TOOLBAR_H),
            fixed_height(find_height),
            LayoutBox::new(Rect::default()).with_sizing(fill),
            fixed_height(STATUS_H),
        ];
        let _ = Column::new(root).arrange(&mut children);
        EditLayout {
            root,
            header: children[0].bounds(),
            toolbar: children[1].bounds(),
            find: children[2].bounds(),
            editor: children[3].bounds(),
            status: children[4].bounds(),
        }
    }

    fn ensure_layout(&mut self) -> bool {
        if !self.layout_invalidation.update(self.client_bounds) {
            return false;
        }
        self.layout = Self::compute_layout(self.client_bounds, self.find.visible);
        true
    }

    fn set_client_bounds(&mut self, width: u32, height: u32) -> bool {
        let bounds = Rect::new(0, 0, width, height);
        if bounds == self.client_bounds {
            return false;
        }
        self.client_bounds = bounds;
        self.layout_invalidation.invalidate();
        let changed = self.ensure_layout();
        self.revalidate_editor_scroll();
        self.layout_find_panel();
        self.layout_dialog_buttons();
        changed
    }

    fn invalidate_structure(&mut self) {
        self.layout_invalidation.invalidate();
        let _ = self.ensure_layout();
        self.revalidate_editor_scroll();
    }

    fn revalidate_editor_scroll(&mut self) {
        self.ensure_cursor_visible();
        let max_scroll = self
            .buffer
            .line_count()
            .saturating_sub(self.visible_line_count());
        self.scroll_line = self.scroll_line.min(max_scroll);
        self.selection.scroll_line = self.scroll_line;
    }

    fn display_name(&self) -> &str {
        if let Some(path) = &self.user_path {
            return path.file_name();
        }
        if self.is_temporary {
            return UNTITLED_DISPLAY;
        }
        self.backing_path
            .as_ref()
            .map(PathBuf::file_name)
            .unwrap_or(UNTITLED_DISPLAY)
    }

    fn refresh_header_title(&mut self) {
        let mut title = String::from("\u{1F31E} ");
        title.push_str(self.display_name());
        if self.buffer.is_dirty() {
            title.push('*');
        }
        self.header_title.set(&title);
    }

    fn refresh_status_left(&mut self) {
        let mut text = String::from("Ln ");
        push_usize(&mut text, self.buffer.cursor_line + 1);
        text.push_str(", Col ");
        push_usize(&mut text, self.buffer.cursor_col + 1);
        if let Some(range) = self.selection_range() {
            text.push_str(" | Sel ");
            push_usize(&mut text, self.range_char_count(range));
        }
        self.status_left.set(&text);
    }

    fn refresh_status_right(&mut self) {
        let mut text = String::new();
        push_usize(&mut text, self.buffer.char_count());
        text.push_str(" chars | ");
        push_usize(&mut text, self.buffer.word_count());
        text.push_str(" words | ");
        push_usize(&mut text, self.buffer.line_count());
        text.push_str(" lines | UTF-8 | ");
        if self.is_temporary {
            text.push_str("Temporary · ");
        }
        if self.buffer.is_dirty() {
            text.push_str("Modified via VFS");
        } else {
            text.push_str("Saved via VFS");
        }
        text.push_str(" | SunlightOS");
        self.status_right.set(&text);
    }

    fn refresh_status_bars(&mut self) {
        self.refresh_status_left();
        self.refresh_status_right();
        self.refresh_header_title();
    }

    fn set_status_message(&mut self, text: &str) {
        self.status_center.set(text);
        self.status_msg_ticks = 30;
    }

    fn note_document_changed(&mut self) {
        self.document_revision = self.document_revision.wrapping_add(1);
        self.selection.preferred_col = None;
        self.refresh_status_bars();
        self.invalidate_find_matches();
    }

    fn invalidate_find_matches(&mut self) {
        self.find.doc_revision = 0;
    }

    fn open_real_file(&mut self, path: PathBuf) {
        self.startup_error.clear();
        match read_utf8_file(path.as_bytes()) {
            Ok(content) => {
                self.buffer = TextBuffer::from_utf8(&content);
                self.backing_path = Some(path);
                self.user_path = Some(path);
                self.is_temporary = false;
                self.scroll_line = 0;
                self.clear_selection();
                self.buffer.mark_saved();
                self.document_revision = self.document_revision.wrapping_add(1);
                self.focus = FocusTarget::Editor;
                self.find.current_match = None;
                self.set_status_message("Opened");
            }
            Err(msg) => {
                self.startup_error.set(msg);
                self.set_status_message(msg);
            }
        }
        self.refresh_status_bars();
    }

    fn create_temp_document(&mut self) {
        self.startup_error.clear();
        match create_temp_backing_path() {
            Ok(path) => {
                self.buffer = TextBuffer::new();
                self.backing_path = Some(path);
                self.user_path = None;
                self.is_temporary = true;
                self.scroll_line = 0;
                self.clear_selection();
                self.buffer.mark_saved();
                self.document_revision = self.document_revision.wrapping_add(1);
            }
            Err(msg) => {
                self.startup_error.set(msg);
                self.set_status_message(msg);
            }
        }
        self.refresh_status_bars();
    }

    fn initialize_document(&mut self) {
        if self.document_ready {
            return;
        }
        self.document_ready = true;
        if let Some(path) = self.pending_user_path.take() {
            self.open_real_file(path);
        } else {
            self.create_temp_document();
        }
    }

    fn save_to_backing(&mut self) -> Result<(), &'static str> {
        let backing = self.backing_path.ok_or("No save path")?;
        let content = self.buffer.to_utf8_string();
        write_utf8_file(backing.as_bytes(), content.as_bytes())
    }

    fn save(&mut self) {
        if self.is_temporary {
            let _ = self.save_as(false);
            return;
        }
        match self.save_to_backing() {
            Ok(()) => {
                self.buffer.mark_saved();
                self.set_status_message("Saved");
            }
            Err(msg) => self.set_status_message(msg),
        }
        self.refresh_status_bars();
    }

    fn save_as(&mut self, close_after_save: bool) -> bool {
        let request = DialogRequest::SaveFile(SaveFileRequest {
            title: String::from("Save File"),
            initial_dir: Some(self.dialog_initial_dir()),
            suggested_name: Some(self.dialog_suggested_name()),
            default_extension: self.dialog_default_extension(),
            allowed_extensions: Vec::new(),
            overwrite_confirm: true,
            confirm_button_label: Some(String::from("Save")),
        });
        match show_dialog(&request) {
            Ok(DialogResult::SavePathSelected(path)) => {
                let Some(path) = PathBuf::from_str(&path) else {
                    self.set_status_message("Selected path is invalid");
                    return false;
                };
                match self.save_to_final_path(path) {
                    Ok(()) => {
                        self.set_status_message("Saved");
                        self.refresh_status_bars();
                        if close_after_save {
                            request_close();
                        }
                        true
                    }
                    Err(msg) => {
                        self.set_status_message(msg);
                        false
                    }
                }
            }
            Ok(DialogResult::Cancelled | DialogResult::Cancel | DialogResult::Dismissed) => {
                self.set_status_message("Save cancelled");
                false
            }
            Ok(DialogResult::Error(message)) => {
                self.set_status_message(&message);
                false
            }
            Ok(_) => {
                self.set_status_message("Save dialog returned unexpected result");
                false
            }
            Err(message) => {
                self.set_status_message(message);
                false
            }
        }
    }

    fn save_to_final_path(&mut self, path: PathBuf) -> Result<(), &'static str> {
        let content = self.buffer.to_utf8_string();
        write_utf8_file(path.as_bytes(), content.as_bytes())?;
        self.backing_path = Some(path);
        self.user_path = Some(path);
        self.is_temporary = false;
        self.buffer.mark_saved();
        Ok(())
    }

    fn needs_close_prompt(&self) -> bool {
        if self.is_temporary {
            !self.buffer.is_content_empty() || self.buffer.is_dirty()
        } else {
            self.buffer.is_dirty()
        }
    }

    fn try_close(&mut self) {
        if self.active_dialog != ActiveDialog::None {
            return;
        }
        if !self.needs_close_prompt() {
            request_close();
            return;
        }
        if self.is_temporary {
            self.active_dialog = ActiveDialog::SaveBeforeCloseTemporary;
            self.layout_dialog_buttons();
        } else {
            self.active_dialog = ActiveDialog::SaveBeforeClose;
            self.layout_dialog_buttons();
        }
    }

    fn dismiss_dialog(&mut self) {
        self.active_dialog = ActiveDialog::None;
        self.dialog_button_count = 0;
    }

    fn dialog_action(&mut self, action: DialogAction) -> bool {
        match (self.active_dialog, action) {
            (ActiveDialog::SaveBeforeClose, DialogAction::Save) => {
                if self.save_to_backing().is_ok() {
                    self.buffer.mark_saved();
                    self.dismiss_dialog();
                    request_close();
                    return true;
                }
                self.set_status_message("Save failed");
                false
            }
            (ActiveDialog::SaveBeforeClose, DialogAction::Discard) => {
                self.dismiss_dialog();
                request_close();
                true
            }
            (ActiveDialog::SaveBeforeClose, DialogAction::Cancel) => {
                self.dismiss_dialog();
                true
            }
            (ActiveDialog::SaveBeforeCloseTemporary, DialogAction::Save) => {
                self.dismiss_dialog();
                self.save_as(true)
            }
            (ActiveDialog::SaveBeforeCloseTemporary, DialogAction::Discard) => {
                self.dismiss_dialog();
                request_close();
                true
            }
            (ActiveDialog::SaveBeforeCloseTemporary, DialogAction::Cancel) => {
                self.dismiss_dialog();
                true
            }
            _ => false,
        }
    }

    fn dialog_panel_rect(&self) -> Rect {
        let desired_h = match self.active_dialog {
            ActiveDialog::SaveBeforeClose | ActiveDialog::SaveBeforeCloseTemporary => 132,
            ActiveDialog::None => 0,
        };
        let w = DIALOG_W.min(self.layout.root.w);
        let h = desired_h.min(self.layout.root.h);
        Rect::new(
            self.layout.root.x + ((self.layout.root.w.saturating_sub(w)) / 2) as i32,
            self.layout.root.y + ((self.layout.root.h.saturating_sub(h)) / 2) as i32,
            w,
            h,
        )
    }

    fn layout_dialog_buttons(&mut self) {
        let panel = self.dialog_panel_rect();
        let specs: [(&str, DialogAction); 3] = match self.active_dialog {
            ActiveDialog::SaveBeforeClose | ActiveDialog::SaveBeforeCloseTemporary => [
                ("Save", DialogAction::Save),
                ("Discard", DialogAction::Discard),
                ("Cancel", DialogAction::Cancel),
            ],
            ActiveDialog::None => [
                ("", DialogAction::Cancel),
                ("", DialogAction::Cancel),
                ("", DialogAction::Cancel),
            ],
        };
        let count = match self.active_dialog {
            ActiveDialog::None => 0,
            _ => 3,
        };
        let total_w =
            count as u32 * DIALOG_BTN_W + (count as u32).saturating_sub(1) * DIALOG_BTN_GAP;
        let mut x = panel.x + ((panel.w as i32 - total_w as i32) / 2);
        let y = panel.bottom() - DIALOG_PAD - DIALOG_BTN_H as i32;
        self.dialog_button_count = count;
        for (i, (label, action)) in specs.iter().enumerate().take(count) {
            self.dialog_buttons[i] = DialogButton {
                action: *action,
                rect: Rect::new(x, y, DIALOG_BTN_W, DIALOG_BTN_H),
                label,
            };
            x += DIALOG_BTN_W as i32 + DIALOG_BTN_GAP as i32;
        }
    }

    fn dialog_button_hit(&self, x: i32, y: i32) -> Option<DialogAction> {
        for btn in &self.dialog_buttons[..self.dialog_button_count] {
            if btn.rect.contains(Point::new(x, y)) {
                return Some(btn.action);
            }
        }
        None
    }

    fn handle_dialog_key(&mut self, keycode: u8, pressed: bool) -> bool {
        if !pressed {
            return false;
        }
        match keycode {
            KEY_ESC => self.dialog_action(DialogAction::Cancel),
            KEY_ENTER => self.dialog_action(DialogAction::Save),
            _ => false,
        }
    }

    fn find_panel_rect(&self) -> Rect {
        self.layout.find
    }

    fn editor_rect(&self) -> Rect {
        self.layout.editor
    }

    fn visible_line_count(&self) -> usize {
        let rect = self.editor_rect();
        let lh = line_height(FontRole::MonoRegular).max(1) as u32;
        (rect.h / lh).max(1) as usize
    }

    fn ensure_cursor_visible(&mut self) {
        let visible = self.visible_line_count();
        if self.buffer.cursor_line < self.scroll_line {
            self.scroll_line = self.buffer.cursor_line;
        } else if self.buffer.cursor_line >= self.scroll_line + visible {
            self.scroll_line = self.buffer.cursor_line + 1 - visible;
        }
    }

    fn update_shared_editor(&mut self, event: Event) -> bool {
        self.selection.scroll_line = self.scroll_line;
        self.selection.focused = self.focus == FocusTarget::Editor;
        let rect = self.editor_rect();
        let response = TextEditor::new(rect, &mut self.buffer, &mut self.selection)
            .with_font(&F_MONO)
            .with_gutter_width(GUTTER_W)
            .with_menu_bounds(self.layout.root)
            .with_clipboard_source(CLIP_SOURCE_APP)
            .update(event);
        self.scroll_line = self.selection.scroll_line;
        self.apply_editor_response(response)
    }

    fn run_shared_editor_command(&mut self, command: TextCommand) -> bool {
        self.selection.scroll_line = self.scroll_line;
        self.selection.focused = true;
        let rect = self.editor_rect();
        let response = TextEditor::new(rect, &mut self.buffer, &mut self.selection)
            .with_font(&F_MONO)
            .with_gutter_width(GUTTER_W)
            .with_menu_bounds(self.layout.root)
            .with_clipboard_source(CLIP_SOURCE_APP)
            .command(command);
        self.scroll_line = self.selection.scroll_line;
        self.apply_editor_response(response)
    }

    fn apply_editor_response(&mut self, response: TextEditorResponse) -> bool {
        if let Some(error) = response.clipboard_error {
            self.set_status_message(error.message());
        } else if let Some(command) = response.command {
            let status = match command {
                TextCommand::Cut => "Cut",
                TextCommand::Copy => "Copied",
                TextCommand::Paste => "Pasted",
                TextCommand::Delete => "Deleted",
                TextCommand::SelectAll => "Selected all",
                TextCommand::Undo => "Undo unavailable",
                TextCommand::Redo => "Redo unavailable",
            };
            self.set_status_message(status);
        }
        if response.changed {
            self.note_document_changed();
        } else if response.selection_changed {
            self.refresh_status_bars();
        }
        response.consumed || response.changed || response.selection_changed
    }

    fn toolbar_rect(&self) -> Rect {
        self.layout.toolbar
    }

    fn toolbar_buttons(&self) -> [ToolbarButton; 6] {
        let left = [
            ToolbarButtonSpec {
                action: EditorAction::Open,
                label: "Open",
            },
            ToolbarButtonSpec {
                action: EditorAction::Save,
                label: "Save",
            },
            ToolbarButtonSpec {
                action: EditorAction::SaveAs,
                label: "Save As",
            },
            ToolbarButtonSpec {
                action: EditorAction::Find,
                label: "Find",
            },
            ToolbarButtonSpec {
                action: EditorAction::Replace,
                label: "Replace",
            },
            ToolbarButtonSpec {
                action: EditorAction::About,
                label: "", // now rendered as hamburger Material icon
            },
        ];
        let rect = self.toolbar_rect();
        let y = rect.y + (rect.h as i32 - TOOLBAR_BTN_W as i32) / 2;
        let mut buttons = [ToolbarButton {
            spec: left[0],
            rect: Rect::new(0, 0, 0, 0),
        }; 6];
        let mut x = rect.x + PAD;
        for (idx, spec) in left.iter().enumerate().take(5) {
            buttons[idx] = ToolbarButton {
                spec: *spec,
                rect: Rect::new(x, y, TOOLBAR_BTN_W, TOOLBAR_BTN_W),
            };
            x += TOOLBAR_BTN_W as i32 + TOOLBAR_GAP;
        }
        buttons[5] = ToolbarButton {
            spec: left[5],
            rect: Rect::new(
                rect.right() - PAD - TOOLBAR_BTN_W as i32,
                y,
                TOOLBAR_BTN_W,
                TOOLBAR_BTN_W,
            ),
        };
        buttons
    }

    fn toolbar_hit(&self, x: i32, y: i32) -> Option<usize> {
        let point = Point::new(x, y);
        self.toolbar_buttons()
            .iter()
            .position(|button| button.rect.contains(point))
    }

    fn layout_find_panel(&mut self) {
        let panel = self.find_panel_rect();
        let input_w = 240u32;
        let input_h = 28u32;
        let y = panel.y + 10;
        self.find.query.rect = Rect::new(80, y, input_w, input_h);
        self.find.replace.rect = Rect::new(80, y + 34, input_w, input_h);
        self.find.query.active = self.find.focus == FocusTarget::Find;
        self.find.replace.active = self.find.focus == FocusTarget::Replace;
    }

    fn update_find_inputs(&mut self, event: Event) -> bool {
        if !self.find.visible {
            return false;
        }
        self.layout_find_panel();
        let query_changed = self.find.query.update(event);
        let replace_changed = self.find.replace_visible && self.find.replace.update(event);
        if query_changed {
            self.find.query_revision = self.find.query_revision.wrapping_add(1);
            self.sync_find_matches();
        }
        query_changed || replace_changed
    }

    fn menu_specs() -> &'static [MenuItemSpec] {
        const HAMBURGER: &[MenuItemSpec] = &[
            MenuItemSpec {
                action: EditorAction::New,
                label: "New",
                icon: Some(ICON_NEW_TGA),
            },
            MenuItemSpec {
                action: EditorAction::Open,
                label: "Open",
                icon: Some(ICON_OPEN_TGA),
            },
            MenuItemSpec {
                action: EditorAction::Save,
                label: "Save",
                icon: Some(ICON_SAVE_TGA),
            },
            MenuItemSpec {
                action: EditorAction::SaveAs,
                label: "Save As",
                icon: Some(ICON_SAVE_AS_TGA),
            },
            MenuItemSpec {
                action: EditorAction::Find,
                label: "Find",
                icon: Some(ICON_FIND_TGA),
            },
            MenuItemSpec {
                action: EditorAction::Replace,
                label: "Replace",
                icon: Some(ICON_REPLACE_TGA),
            },
            MenuItemSpec {
                action: EditorAction::SelectAll,
                label: "Select All",
                icon: Some(ICON_SELECT_ALL_TGA),
            },
            MenuItemSpec {
                action: EditorAction::Cut,
                label: "Cut",
                icon: Some(ICON_CUT_TGA),
            },
            MenuItemSpec {
                action: EditorAction::Copy,
                label: "Copy",
                icon: Some(ICON_COPY_TGA),
            },
            MenuItemSpec {
                action: EditorAction::Paste,
                label: "Paste",
                icon: Some(ICON_PASTE_TGA),
            },
            MenuItemSpec {
                action: EditorAction::About,
                label: "Editor Info",
                icon: None,
            },
        ];
        HAMBURGER
    }

    fn open_menu(&mut self, x: i32, y: i32) {
        let specs = Self::menu_specs();
        let menu_h = MENU_ITEM_H * specs.len() as u32 + 8;
        let max_x = self.layout.root.right() - MENU_W as i32 - 6;
        let max_y = self.layout.status.y - menu_h as i32 - 6;
        let rect = Rect::new(
            x.clamp(6, max_x.max(6)),
            y.clamp(6, max_y.max(6)),
            MENU_W,
            menu_h,
        );
        let mut items = [MenuItem {
            spec: MenuItemSpec {
                action: EditorAction::About,
                label: "",
                icon: None,
            },
            rect: Rect::new(0, 0, 0, 0),
            enabled: false,
        }; 12];
        for (i, spec) in specs.iter().enumerate() {
            items[i] = MenuItem {
                spec: *spec,
                rect: Rect::new(
                    rect.x + 4,
                    rect.y + 4 + i as i32 * MENU_ITEM_H as i32,
                    MENU_W - 8,
                    MENU_ITEM_H,
                ),
                enabled: self.action_enabled(spec.action),
            };
        }
        self.menu = Some(PopupMenu {
            rect,
            items,
            count: specs.len(),
        });
    }

    fn close_menu(&mut self) {
        self.menu = None;
    }

    fn selection_range(&self) -> Option<TextRange> {
        let anchor = self.selection.anchor?;
        let caret = self.buffer.cursor();
        let range = self.buffer.normalized_range(anchor, caret);
        if range.start == range.end {
            None
        } else {
            Some(range)
        }
    }

    fn clear_selection(&mut self) {
        self.selection.anchor = None;
        self.selection.drag_anchor = None;
        self.selection.drag_active = false;
    }

    fn has_selection(&self) -> bool {
        self.selection_range().is_some()
    }

    fn range_char_count(&self, range: TextRange) -> usize {
        self.buffer
            .extract_range(range.start, range.end)
            .chars()
            .count()
    }

    fn delete_selection(&mut self) -> bool {
        let Some(range) = self.selection_range() else {
            return false;
        };
        let changed = self.buffer.delete_range(range.start, range.end);
        if changed {
            self.clear_selection();
            self.note_document_changed();
            self.ensure_cursor_visible();
        }
        changed
    }

    fn selected_text(&self) -> Option<String> {
        self.selection_range()
            .map(|range| self.buffer.extract_range(range.start, range.end))
    }

    fn action_enabled(&self, action: EditorAction) -> bool {
        match action {
            EditorAction::Cut | EditorAction::Copy => self.has_selection(),
            EditorAction::ReplaceCurrent => self.find.current_match.is_some(),
            EditorAction::ReplaceAll => !self.find.query.value().is_empty(),
            EditorAction::FindNext | EditorAction::FindPrev => !self.find.query.value().is_empty(),
            _ => true,
        }
    }

    fn dispatch_action(&mut self, action: EditorAction) -> bool {
        match action {
            EditorAction::New => {
                if self.has_unsaved_content() && !self.confirm_discard_for_open() {
                    self.set_status_message("New cancelled");
                    return false;
                }
                self.create_temp_document();
                self.find.visible = false;
                self.invalidate_structure();
                self.close_menu();
                self.set_status_message("New document");
                true
            }
            EditorAction::Open => {
                self.close_menu();
                self.open_with_dialog();
                true
            }
            EditorAction::Save => {
                self.close_menu();
                self.save();
                true
            }
            EditorAction::SaveAs => {
                self.close_menu();
                let _ = self.save_as(false);
                true
            }
            EditorAction::Find => {
                self.close_menu();
                self.show_find(false);
                true
            }
            EditorAction::Replace => {
                self.close_menu();
                self.show_find(true);
                true
            }
            EditorAction::FindNext => self.find_next(false),
            EditorAction::FindPrev => self.find_next(true),
            EditorAction::ReplaceCurrent => self.replace_current_match(),
            EditorAction::ReplaceAll => self.replace_all_matches(),
            EditorAction::SelectAll => {
                self.close_menu();
                self.run_shared_editor_command(TextCommand::SelectAll)
            }
            EditorAction::Cut => {
                self.close_menu();
                self.run_shared_editor_command(TextCommand::Cut)
            }
            EditorAction::Copy => {
                self.close_menu();
                self.run_shared_editor_command(TextCommand::Copy)
            }
            EditorAction::Paste => {
                self.close_menu();
                self.run_shared_editor_command(TextCommand::Paste)
            }
            EditorAction::About => {
                self.close_menu();
                self.set_status_message("Sunlight Edit · UTF-8 text editor");
                true
            }
        }
    }

    fn handle_navigation(&mut self, keycode: u8, shift: bool, ctrl: bool) -> bool {
        let anchor_before = self.buffer.cursor();
        let changed = if ctrl {
            match keycode {
                KEY_LEFT => self.buffer.move_word_left(),
                KEY_RIGHT => self.buffer.move_word_right(),
                KEY_HOME => self.buffer.move_document_home(),
                KEY_END => self.buffer.move_document_end(),
                _ => false,
            }
        } else {
            match keycode {
                KEY_LEFT => self.buffer.move_left(),
                KEY_RIGHT => self.buffer.move_right(),
                KEY_UP => {
                    let preferred = self
                        .selection
                        .preferred_col
                        .unwrap_or(self.buffer.cursor_col);
                    let moved = self.buffer.move_up();
                    if moved {
                        self.buffer.cursor_col =
                            preferred.min(self.buffer.line_len_chars(self.buffer.cursor_line));
                        self.selection.preferred_col = Some(preferred);
                    }
                    moved
                }
                KEY_DOWN => {
                    let preferred = self
                        .selection
                        .preferred_col
                        .unwrap_or(self.buffer.cursor_col);
                    let moved = self.buffer.move_down();
                    if moved {
                        self.buffer.cursor_col =
                            preferred.min(self.buffer.line_len_chars(self.buffer.cursor_line));
                        self.selection.preferred_col = Some(preferred);
                    }
                    moved
                }
                KEY_HOME => self.buffer.move_home(),
                KEY_END => self.buffer.move_end(),
                _ => false,
            }
        };
        if !changed {
            return false;
        }
        if !matches!(keycode, KEY_UP | KEY_DOWN) {
            self.selection.preferred_col = None;
        }
        if shift {
            self.selection.anchor.get_or_insert(anchor_before);
        } else {
            self.clear_selection();
        }
        self.ensure_cursor_visible();
        self.refresh_status_bars();
        true
    }

    fn handle_key_press(&mut self, keycode: u8, pressed: bool, shift: bool, ctrl: bool) -> bool {
        if self.active_dialog != ActiveDialog::None {
            return self.handle_dialog_key(keycode, pressed);
        }
        if !pressed {
            return false;
        }
        if keycode == KEY_ESC {
            if self.menu.is_some() {
                self.close_menu();
                return true;
            }
            if self.find.visible {
                self.hide_find();
                return true;
            }
            self.try_close();
            return true;
        }
        if self.focus == FocusTarget::Find || self.focus == FocusTarget::Replace {
            if self.handle_find_keypress(keycode, shift, ctrl) {
                return true;
            }
        }
        if self.focus == FocusTarget::Editor
            && self.update_shared_editor(Event::KeyPress {
                keycode,
                pressed: true,
                shift,
                ctrl,
                alt: false,
                super_key: false,
            })
        {
            return true;
        }
        if ctrl {
            let action = match keycode {
                KEY_O => Some(EditorAction::Open),
                KEY_S if shift => Some(EditorAction::SaveAs),
                KEY_S => Some(EditorAction::Save),
                KEY_F => Some(EditorAction::Find),
                KEY_H => Some(EditorAction::Replace),
                KEY_A => Some(EditorAction::SelectAll),
                KEY_C => Some(EditorAction::Copy),
                KEY_X => Some(EditorAction::Cut),
                KEY_V => Some(EditorAction::Paste),
                KEY_Y => None,
                KEY_Z => None,
                _ => None,
            };
            if let Some(action) = action {
                return self.dispatch_action(action);
            }
        }
        if self.handle_navigation(keycode, shift, ctrl) {
            return true;
        }
        match keycode {
            KEY_DELETE => {
                if self.delete_selection() {
                    return true;
                }
                let changed = self.buffer.delete_forward();
                if changed {
                    self.note_document_changed();
                    self.ensure_cursor_visible();
                }
                changed
            }
            _ => false,
        }
    }

    fn handle_find_keypress(&mut self, keycode: u8, shift: bool, ctrl: bool) -> bool {
        if ctrl {
            return self.update_find_inputs(Event::KeyPress {
                keycode,
                pressed: true,
                shift,
                ctrl: true,
                alt: false,
                super_key: false,
            });
        }
        match keycode {
            KEY_ENTER => {
                if shift {
                    self.find_next(true)
                } else {
                    self.find_next(false)
                }
            }
            KEY_ESC => {
                self.hide_find();
                true
            }
            KEY_UP | KEY_DOWN => {
                self.focus = if self.focus == FocusTarget::Find && self.find.replace_visible {
                    FocusTarget::Replace
                } else {
                    FocusTarget::Find
                };
                self.layout_find_panel();
                true
            }
            _ => {
                let event = Event::KeyPress {
                    keycode,
                    pressed: true,
                    shift,
                    ctrl: false,
                    alt: false,
                    super_key: false,
                };
                let changed = if self.focus == FocusTarget::Find {
                    self.find.query.update(event)
                } else {
                    self.find.replace.update(event)
                };
                if changed {
                    if self.focus == FocusTarget::Find {
                        self.find.query_revision = self.find.query_revision.wrapping_add(1);
                    }
                    self.sync_find_matches();
                }
                changed
            }
        }
    }

    fn handle_text_key(&mut self, ch: char) -> bool {
        if self.active_dialog != ActiveDialog::None {
            return false;
        }
        if self.focus == FocusTarget::Find || self.focus == FocusTarget::Replace {
            let changed = if self.focus == FocusTarget::Find {
                self.find.query.update(Event::Key(ch))
            } else {
                self.find.replace.update(Event::Key(ch))
            };
            if changed {
                if self.focus == FocusTarget::Find {
                    self.find.query_revision = self.find.query_revision.wrapping_add(1);
                    self.sync_find_matches();
                }
                return true;
            }
            return false;
        }
        self.update_shared_editor(Event::Key(ch))
    }

    fn show_find(&mut self, replace: bool) {
        let was_visible = self.find.visible;
        self.find.visible = true;
        self.find.replace_visible = replace;
        if let Some(text) = self.selected_text() {
            if !text.is_empty() && !text.contains('\n') && text.len() < FIND_QUERY_MAX {
                self.find.query.set_text(&text);
                self.find.query_revision = self.find.query_revision.wrapping_add(1);
            }
        }
        self.focus = FocusTarget::Find;
        self.find.focus = FocusTarget::Find;
        if !was_visible {
            self.invalidate_structure();
        }
        self.layout_find_panel();
        self.sync_find_matches();
    }

    fn hide_find(&mut self) {
        let was_visible = self.find.visible;
        self.find.visible = false;
        self.find.replace_visible = false;
        self.focus = FocusTarget::Editor;
        self.find.focus = FocusTarget::Editor;
        self.find.current_match = None;
        if was_visible {
            self.invalidate_structure();
        }
        self.layout_find_panel();
    }

    fn sync_find_matches(&mut self) {
        if !self.find.visible {
            return;
        }
        let query = self.find.query.value();
        if query.is_empty() {
            self.find.matches.clear();
            self.find.current_match = None;
            return;
        }
        if self.find.doc_revision == self.document_revision {
            return;
        }
        self.find.matches = self.buffer.find_all(query);
        self.find.doc_revision = self.document_revision;
        if self.find.matches.is_empty() {
            self.find.current_match = None;
            self.set_status_message("No matches");
        } else {
            self.find.current_match = Some(0);
            let range = self.find.matches[0];
            self.buffer.set_cursor(range.end);
            self.selection.anchor = Some(range.start);
            self.ensure_cursor_visible();
        }
    }

    fn find_next(&mut self, previous: bool) -> bool {
        if !self.find.visible {
            self.show_find(false);
        }
        let query = self.find.query.value();
        if query.is_empty() {
            self.set_status_message("Find query is empty");
            return false;
        }
        self.find.doc_revision = 0;
        self.sync_find_matches();
        if self.find.matches.is_empty() {
            return false;
        }
        let len = self.find.matches.len();
        let current = self.find.current_match.unwrap_or(0);
        let next = if previous {
            if current == 0 {
                len - 1
            } else {
                current - 1
            }
        } else {
            (current + 1) % len
        };
        self.find.current_match = Some(next);
        let range = self.find.matches[next];
        self.buffer.set_cursor(range.end);
        self.selection.anchor = Some(range.start);
        self.ensure_cursor_visible();
        self.refresh_status_bars();
        true
    }

    fn replace_current_match(&mut self) -> bool {
        self.find.doc_revision = 0;
        self.sync_find_matches();
        let Some(index) = self.find.current_match else {
            self.set_status_message("No match selected");
            return false;
        };
        if index >= self.find.matches.len() {
            return false;
        }
        let replacement = String::from(self.find.replace.value());
        let range = self.find.matches[index];
        let changed = self
            .buffer
            .replace_range(range.start, range.end, &replacement);
        if changed {
            self.clear_selection();
            self.note_document_changed();
            self.find.doc_revision = 0;
            self.sync_find_matches();
            self.set_status_message("Replaced match");
        }
        changed
    }

    fn replace_all_matches(&mut self) -> bool {
        let query = String::from(self.find.query.value());
        if query.is_empty() {
            self.set_status_message("Find query is empty");
            return false;
        }
        let replacement = String::from(self.find.replace.value());
        let matches = self.buffer.find_all(&query);
        if matches.is_empty() {
            self.set_status_message("No matches");
            return false;
        }
        let text = self.buffer.to_utf8_string().replace(&query, &replacement);
        self.buffer = TextBuffer::from_utf8(&text);
        self.clear_selection();
        self.note_document_changed();
        self.find.doc_revision = 0;
        self.sync_find_matches();
        self.set_status_message("Replaced all");
        true
    }

    fn draw_header(&self, canvas: &mut Canvas, theme: &Theme) {
        let rect = self.layout.header;
        canvas.fill_rect(rect, theme.panel);
        canvas.hbar(rect.x, rect.bottom() - 1, rect.w, 1, theme.border);
        draw_text_vcenter(
            canvas,
            self.header_title.as_str(),
            PAD,
            rect.y,
            rect.h,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );
        draw_text_vcenter(
            canvas,
            "Sunlight Edit",
            rect.right() - 120,
            rect.y,
            rect.h,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
    }

    fn draw_toolbar_button(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        idx: usize,
        button: ToolbarButton,
    ) {
        let state = if self.toolbar_pressed == Some(idx) {
            ButtonState::Pressed
        } else if self.toolbar_hover == Some(idx) {
            ButtonState::Hovered
        } else {
            ButtonState::Normal
        };
        let bg = match state {
            ButtonState::Hovered => theme.panel_alt,
            ButtonState::Pressed => theme.border,
            _ => theme.panel,
        };
        canvas.fill_rounded_rect(button.rect, 6, bg);
        canvas.stroke_rounded_rect(button.rect, 6, 1, theme.border);
        if let Some(icon) = self.icons.icon_for(button.spec.action) {
            // Use tinted monochrome Material icon so it matches theme (monochrome action icons).
            canvas.draw_tga_icon_tinted(
                icon,
                Rect::new(
                    button.rect.x + 12,
                    button.rect.y + 12,
                    TOOLBAR_ICON,
                    TOOLBAR_ICON,
                ),
                theme.icon_foreground,
            );
        } else {
            draw_text_vcenter(
                canvas,
                button.spec.label,
                button.rect.x + 6,
                button.rect.y,
                button.rect.h,
                &TextStyle::new(FontRole::UiSmall, theme.text),
            );
        }
    }

    fn draw_toolbar(&self, canvas: &mut Canvas, theme: &Theme) {
        let rect = self.toolbar_rect();
        canvas.fill_rect(rect, theme.panel);
        canvas.hbar(rect.x, rect.bottom() - 1, rect.w, 1, theme.border);
        for (idx, button) in self.toolbar_buttons().iter().enumerate() {
            self.draw_toolbar_button(canvas, theme, idx, *button);
        }
    }

    fn draw_match_highlight(&self, canvas: &mut Canvas, theme: &Theme, highlight: MatchHighlight) {
        let rect = self.editor_rect();
        let gutter = Rect::new(rect.x, rect.y, GUTTER_W, rect.h);
        let text_x = gutter.right() + PAD;
        let lh = line_height(FontRole::MonoRegular).max(1) as i32;
        if highlight.range.start.line != highlight.range.end.line {
            return;
        }
        let line_idx = highlight.range.start.line;
        if line_idx < self.scroll_line {
            return;
        }
        let row = line_idx - self.scroll_line;
        let y = rect.y + row as i32 * lh;
        if y >= rect.bottom() {
            return;
        }
        let line = self.buffer.line(line_idx).unwrap_or("");
        let prefix: String = line.chars().take(highlight.range.start.col).collect();
        let selected: String = line
            .chars()
            .skip(highlight.range.start.col)
            .take(
                highlight
                    .range
                    .end
                    .col
                    .saturating_sub(highlight.range.start.col),
            )
            .collect();
        let sx = text_x + measure_text(&prefix, FontRole::MonoRegular).w as i32;
        let sw = measure_text(&selected, FontRole::MonoRegular).w.max(4) as i32;
        let fill = if highlight.current {
            Color::rgba(theme.accent.r(), theme.accent.g(), theme.accent.b(), 110)
        } else {
            Color::rgba(
                theme.text_dim.r(),
                theme.text_dim.g(),
                theme.text_dim.b(),
                60,
            )
        };
        for py in y..(y + lh - 1).min(rect.bottom()) {
            for px in sx..sx + sw {
                canvas.blend_pixel(px, py, fill);
            }
        }
    }

    fn draw_editor(&mut self, canvas: &mut Canvas, theme: &Theme) {
        let rect = self.editor_rect();
        self.selection.scroll_line = self.scroll_line;
        self.selection.focused = self.focus == FocusTarget::Editor;
        {
            let editor = TextEditor::new(rect, &mut self.buffer, &mut self.selection)
                .with_font(&F_MONO)
                .with_caret_visible(self.caret_visible)
                .with_gutter_width(GUTTER_W)
                .with_menu_bounds(self.layout.root)
                .with_clipboard_source(CLIP_SOURCE_APP);
            editor.draw_surface(canvas, theme);
        }
        if self.find.visible {
            for (idx, range) in self.find.matches.iter().enumerate() {
                self.draw_match_highlight(
                    canvas,
                    theme,
                    MatchHighlight {
                        range: *range,
                        current: self.find.current_match == Some(idx),
                    },
                );
            }
        }
    }

    fn draw_editor_context_menu(&mut self, canvas: &mut Canvas, theme: &Theme) {
        let rect = self.editor_rect();
        let editor = TextEditor::new(rect, &mut self.buffer, &mut self.selection)
            .with_font(&F_MONO)
            .with_gutter_width(GUTTER_W)
            .with_menu_bounds(self.layout.root)
            .with_clipboard_source(CLIP_SOURCE_APP);
        editor.draw_context_menu(canvas, theme);
    }

    fn draw_find_panel(&self, canvas: &mut Canvas, theme: &Theme) {
        if !self.find.visible {
            return;
        }
        let rect = self.find_panel_rect();
        canvas.fill_rect(rect, theme.panel_alt);
        canvas.hbar(rect.x, rect.bottom() - 1, rect.w, 1, theme.border);
        draw_text_vcenter(
            canvas,
            "Find:",
            PAD,
            self.find.query.rect.y,
            self.find.query.rect.h,
            &TextStyle::new(FontRole::UiSmall, theme.text),
        );
        self.find.query.draw(canvas, theme);
        if self.find.replace_visible {
            draw_text_vcenter(
                canvas,
                "Replace:",
                PAD,
                self.find.replace.rect.y,
                self.find.replace.rect.h,
                &TextStyle::new(FontRole::UiSmall, theme.text),
            );
            self.find.replace.draw(canvas, theme);
        }
        let mut info = String::new();
        if self.find.query.value().is_empty() {
            info.push_str("Enter text to search");
        } else {
            push_usize(&mut info, self.find.matches.len());
            info.push_str(" matches");
        }
        draw_text_vcenter(
            canvas,
            &info,
            self.find.query.rect.right() + 12,
            self.find.query.rect.y,
            self.find.query.rect.h,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        self.draw_find_action_button(
            canvas,
            theme,
            EditorAction::FindPrev,
            Rect::new(rect.right() - 180, self.find.query.rect.y, 28, 28),
        );
        self.draw_find_action_button(
            canvas,
            theme,
            EditorAction::FindNext,
            Rect::new(rect.right() - 148, self.find.query.rect.y, 28, 28),
        );
        if self.find.replace_visible {
            self.draw_find_action_button(
                canvas,
                theme,
                EditorAction::ReplaceCurrent,
                Rect::new(rect.right() - 116, self.find.replace.rect.y, 52, 28),
            );
            self.draw_find_action_button(
                canvas,
                theme,
                EditorAction::ReplaceAll,
                Rect::new(rect.right() - 60, self.find.replace.rect.y, 52, 28),
            );
        }
    }

    fn draw_find_action_button(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        action: EditorAction,
        rect: Rect,
    ) {
        let enabled = self.action_enabled(action);
        canvas.fill_rounded_rect(
            rect,
            6,
            if enabled {
                theme.panel
            } else {
                theme.panel_alt
            },
        );
        canvas.stroke_rounded_rect(rect, 6, 1, theme.border);
        if let Some(icon) = self.icons.icon_for(action) {
            canvas.draw_tga_icon_tinted(
                icon,
                Rect::new(rect.x + ((rect.w as i32 - 16) / 2), rect.y + 6, 16, 16),
                if enabled {
                    theme.icon_foreground
                } else {
                    theme.icon_disabled
                },
            );
        } else {
            let label = match action {
                EditorAction::ReplaceCurrent => "One",
                EditorAction::ReplaceAll => "All",
                _ => "",
            };
            draw_text_vcenter(
                canvas,
                label,
                rect.x + 4,
                rect.y,
                rect.h,
                &TextStyle::new(
                    FontRole::UiSmall,
                    if enabled { theme.text } else { theme.text_dim },
                ),
            );
        }
    }

    fn draw_startup_error(&self, canvas: &mut Canvas, theme: &Theme) {
        if self.startup_error.len == 0 {
            return;
        }
        let rect = Rect::new(
            self.layout.editor.x + PAD,
            self.layout.editor.y + 4,
            self.layout.editor.w.saturating_sub(16),
            20,
        );
        draw_text(
            canvas,
            self.startup_error.as_str(),
            rect.x,
            rect.y,
            &TextStyle::new(FontRole::UiSmall, theme.danger),
        );
    }

    fn draw_dialog_button(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        btn: DialogButton,
        hovered: bool,
    ) {
        let fill = if hovered {
            theme.panel_alt
        } else {
            theme.panel
        };
        canvas.fill_rounded_rect(btn.rect, 5, fill);
        canvas.stroke_rounded_rect(btn.rect, 5, 1, theme.border);
        draw_text_vcenter(
            canvas,
            btn.label,
            btn.rect.x + 8,
            btn.rect.y,
            btn.rect.h,
            &TextStyle::new(FontRole::UiSmall, theme.text),
        );
    }

    fn draw_active_dialog(&self, canvas: &mut Canvas, theme: &Theme) {
        if self.active_dialog == ActiveDialog::None {
            return;
        }
        canvas.fill_rect(self.layout.root, theme.bg.darken(70));
        let panel = self.dialog_panel_rect();
        canvas.fill_rounded_rect_with_border(panel, 8, theme.panel, theme.border, 1);

        let title = match self.active_dialog {
            ActiveDialog::SaveBeforeClose => "Save changes before closing?",
            ActiveDialog::SaveBeforeCloseTemporary => {
                "Save changes before closing this temporary document?"
            }
            ActiveDialog::None => "",
        };
        draw_text(
            canvas,
            title,
            panel.x + DIALOG_PAD,
            panel.y + DIALOG_PAD,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );

        for btn in &self.dialog_buttons[..self.dialog_button_count] {
            self.draw_dialog_button(canvas, theme, *btn, false);
        }
    }

    fn draw_popup_menu(&self, canvas: &mut Canvas, theme: &Theme) {
        let Some(menu) = self.menu else {
            return;
        };
        canvas.fill_rounded_rect(menu.rect, 8, theme.panel);
        canvas.stroke_rounded_rect(menu.rect, 8, 1, theme.border);
        for item in &menu.items[..menu.count] {
            let item_bg = if item.enabled {
                theme.panel_alt
            } else {
                theme.panel
            };
            canvas.fill_rect(item.rect, item_bg);
            if let Some(icon) = item
                .spec
                .icon
                .and_then(|_| self.icons.icon_for(item.spec.action))
            {
                let col = if item.enabled {
                    theme.icon_foreground
                } else {
                    theme.icon_disabled
                };
                canvas.draw_tga_icon_tinted(
                    icon,
                    Rect::new(item.rect.x + 4, item.rect.y + 4, 16, 16),
                    col,
                );
            }
            draw_text_vcenter(
                canvas,
                item.spec.label,
                item.rect.x + 24,
                item.rect.y,
                item.rect.h,
                &TextStyle::new(
                    FontRole::UiRegular,
                    if item.enabled {
                        theme.text
                    } else {
                        theme.text_dim
                    },
                ),
            );
        }
    }

    fn draw_status(&self, canvas: &mut Canvas, theme: &Theme) {
        let rect = self.layout.status;
        StatusBar::new(
            rect,
            self.status_left.as_str(),
            self.status_center.as_str(),
            self.status_right.as_str(),
        )
        .draw(canvas, theme);
    }

    fn handle_find_panel_click(&mut self, x: i32, y: i32) -> bool {
        if !self.find.visible || !self.find_panel_rect().contains(Point::new(x, y)) {
            return false;
        }
        let prev = Rect::new(
            self.find_panel_rect().right() - 180,
            self.find.query.rect.y,
            28,
            28,
        );
        let next = Rect::new(
            self.find_panel_rect().right() - 148,
            self.find.query.rect.y,
            28,
            28,
        );
        let rep_one = Rect::new(
            self.find_panel_rect().right() - 116,
            self.find.replace.rect.y,
            52,
            28,
        );
        let rep_all = Rect::new(
            self.find_panel_rect().right() - 60,
            self.find.replace.rect.y,
            52,
            28,
        );
        let point = Point::new(x, y);
        if prev.contains(point) {
            return self.dispatch_action(EditorAction::FindPrev);
        }
        if next.contains(point) {
            return self.dispatch_action(EditorAction::FindNext);
        }
        if self.find.replace_visible && rep_one.contains(point) {
            return self.dispatch_action(EditorAction::ReplaceCurrent);
        }
        if self.find.replace_visible && rep_all.contains(point) {
            return self.dispatch_action(EditorAction::ReplaceAll);
        }
        false
    }

    fn show_menu_from_toolbar(&mut self) {
        let button = self.toolbar_buttons()[5];
        self.open_menu(button.rect.x - 120, button.rect.bottom() + 4);
    }
}

impl App for EditApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        if self.client_bounds.size() != Size::new(canvas.width, canvas.height) {
            let _ = self.set_client_bounds(canvas.width, canvas.height);
        } else {
            let _ = self.ensure_layout();
        }
        self.layout_find_panel();
        canvas.fill_rect(self.layout.root, theme.bg);
        self.draw_header(canvas, theme);
        self.draw_toolbar(canvas, theme);
        self.draw_find_panel(canvas, theme);
        self.draw_editor(canvas, theme);
        self.draw_startup_error(canvas, theme);
        self.draw_status(canvas, theme);
        self.draw_popup_menu(canvas, theme);
        self.draw_editor_context_menu(canvas, theme);
        self.draw_active_dialog(canvas, theme);
    }

    fn on_ready(&mut self) -> bool {
        self.initialize_document();
        true
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Tick => {
                let mut redraw = false;
                self.caret_ticks = self.caret_ticks.wrapping_add(1);
                if self.caret_ticks % 5 == 0 {
                    self.caret_visible = !self.caret_visible;
                    redraw = true;
                }
                if self.status_msg_ticks > 0 {
                    self.status_msg_ticks -= 1;
                    if self.status_msg_ticks == 0 {
                        self.status_center.clear();
                        redraw = true;
                    }
                }
                redraw
            }
            Event::Click { x, y } => {
                self.toolbar_pressed = None;
                if self.active_dialog != ActiveDialog::None {
                    if let Some(action) = self.dialog_button_hit(x, y) {
                        return self.dialog_action(action);
                    }
                    return false;
                }
                if self.selection.context_menu_open() {
                    return self.update_shared_editor(Event::Click { x, y });
                }
                if self.find.visible
                    && (self.find.query.context_menu_open()
                        || self.find.replace.context_menu_open())
                {
                    return self.update_find_inputs(Event::Click { x, y });
                }
                if let Some(menu) = self.menu {
                    let p = Point::new(x, y);
                    if menu.rect.contains(p) {
                        if let Some(item) = menu.items[..menu.count]
                            .iter()
                            .find(|item| item.rect.contains(p) && item.enabled)
                        {
                            return self.dispatch_action(item.spec.action);
                        }
                        return true;
                    }
                    self.close_menu();
                    return true;
                }
                if let Some(idx) = self.toolbar_hit(x, y) {
                    if idx == 5 {
                        self.show_menu_from_toolbar();
                        return true;
                    }
                    let buttons = self.toolbar_buttons();
                    return self.dispatch_action(buttons[idx].spec.action);
                }
                if self.handle_find_panel_click(x, y) {
                    return true;
                }
                if self.find.visible {
                    let query_hit = self.find.query.rect.contains(Point::new(x, y));
                    let replace_hit = self.find.replace.rect.contains(Point::new(x, y));
                    self.focus = if query_hit {
                        FocusTarget::Find
                    } else if replace_hit && self.find.replace_visible {
                        FocusTarget::Replace
                    } else {
                        FocusTarget::Editor
                    };
                    self.find.focus = self.focus;
                    self.layout_find_panel();
                    if self.update_find_inputs(Event::Click { x, y }) {
                        return true;
                    }
                }
                if self.update_shared_editor(Event::Click { x, y }) {
                    return true;
                }
                false
            }
            Event::MouseDown { x, y, button } => {
                if self.active_dialog != ActiveDialog::None {
                    return false;
                }
                if button == 0 {
                    self.toolbar_pressed = self.toolbar_hit(x, y);
                }
                if self.editor_rect().contains(Point::new(x, y)) {
                    self.focus = FocusTarget::Editor;
                    self.find.focus = FocusTarget::Editor;
                }
                if self.find.visible {
                    let point = Point::new(x, y);
                    if self.find.query.rect.contains(point) {
                        self.focus = FocusTarget::Find;
                        self.find.focus = FocusTarget::Find;
                    } else if self.find.replace_visible && self.find.replace.rect.contains(point) {
                        self.focus = FocusTarget::Replace;
                        self.find.focus = FocusTarget::Replace;
                    }
                    if self.update_find_inputs(Event::MouseDown { x, y, button }) {
                        return true;
                    }
                }
                self.update_shared_editor(Event::MouseDown { x, y, button })
            }
            Event::MouseUp { x, y, button } => {
                if self.update_find_inputs(Event::MouseUp { x, y, button }) {
                    return true;
                }
                self.update_shared_editor(Event::MouseUp { x, y, button })
            }
            Event::MouseMove { x, y } => {
                if self.active_dialog != ActiveDialog::None {
                    return false;
                }
                let find_redraw = self.update_find_inputs(Event::MouseMove { x, y });
                if self.update_shared_editor(Event::MouseMove { x, y }) {
                    return true;
                }
                let hover = self.toolbar_hit(x, y);
                if hover != self.toolbar_hover {
                    self.toolbar_hover = hover;
                    return true;
                }
                find_redraw
            }
            Event::MouseWheel { .. } => {
                if self.active_dialog != ActiveDialog::None || self.menu.is_some() {
                    return false;
                }
                self.update_shared_editor(event)
            }
            Event::Key(ch) => self.handle_text_key(ch),
            Event::KeyPress {
                keycode,
                pressed,
                shift,
                ctrl,
                ..
            } => self.handle_key_press(keycode, pressed, shift, ctrl),
            Event::FocusChanged { focused: false }
            | Event::PointerOwnership { owned: false, .. } => {
                self.toolbar_pressed = None;
                let find_changed = self.update_find_inputs(event);
                find_changed | self.update_shared_editor(event)
            }
            Event::FocusChanged { focused: true } | Event::PointerOwnership { owned: true, .. } => {
                let find_changed = self.update_find_inputs(event);
                find_changed | self.update_shared_editor(event)
            }
        }
    }

    fn window_event(&mut self, event: WindowEvent) -> bool {
        let WindowEvent::Resized { width, height } = event;
        self.set_client_bounds(width, height)
    }
}

impl EditApp {
    fn dialog_initial_dir(&self) -> String {
        self.user_path
            .or(self.backing_path)
            .and_then(path_parent_string)
            .unwrap_or_else(|| String::from(user_home_dir()))
    }

    fn dialog_suggested_name(&self) -> String {
        if let Some(path) = self.user_path {
            return String::from(path.file_name());
        }
        String::from("untitled.txt")
    }

    fn dialog_default_extension(&self) -> Option<String> {
        let suggested = self.dialog_suggested_name();
        if suggested
            .rsplit('/')
            .next()
            .unwrap_or(&suggested)
            .contains('.')
        {
            None
        } else {
            Some(String::from("txt"))
        }
    }

    fn has_unsaved_content(&self) -> bool {
        if self.is_temporary {
            !self.buffer.is_content_empty() || self.buffer.is_dirty()
        } else {
            self.buffer.is_dirty()
        }
    }

    fn open_with_dialog(&mut self) {
        if self.has_unsaved_content() && !self.confirm_discard_for_open() {
            self.set_status_message("Open cancelled");
            return;
        }
        let request = DialogRequest::OpenFile(OpenFileRequest {
            title: String::from("Open File"),
            initial_dir: Some(self.dialog_initial_dir()),
            allowed_mime_types: Vec::new(),
            allowed_extensions: Vec::new(),
            allow_multiple: false,
            show_preview: true,
            confirm_button_label: Some(String::from("Open")),
        });
        match show_dialog(&request) {
            Ok(DialogResult::FileSelected(path)) => {
                let Some(path) = PathBuf::from_str(&path) else {
                    self.set_status_message("Selected path is invalid");
                    return;
                };
                self.open_real_file(path);
            }
            Ok(DialogResult::Cancelled | DialogResult::Cancel | DialogResult::Dismissed) => {
                self.set_status_message("Open cancelled");
            }
            Ok(DialogResult::Error(message)) => self.set_status_message(&message),
            Ok(_) => self.set_status_message("Open dialog returned unexpected result"),
            Err(message) => self.set_status_message(message),
        }
    }

    fn confirm_discard_for_open(&mut self) -> bool {
        let request = DialogRequest::Confirm(ConfirmRequest {
            common: DialogCommonOptions {
                title: String::from("Discard Changes?"),
                message: String::from("Opening another file will discard unsaved changes."),
                severity: sunlight_dialogs::DialogSeverity::Question,
                silent: false,
            },
            style: ConfirmStyle::OkCancel,
            default_button: DialogChoiceButton::Cancel,
        });
        match show_dialog(&request) {
            Ok(DialogResult::Ok | DialogResult::Yes) => true,
            Ok(DialogResult::Error(message)) => {
                self.set_status_message(&message);
                false
            }
            Err(message) => {
                self.set_status_message(message);
                false
            }
            _ => false,
        }
    }
}

fn push_usize(out: &mut String, mut n: usize) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut digits = [0u8; 20];
    let mut len = 0usize;
    while n > 0 {
        digits[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
    }
    while len > 0 {
        len -= 1;
        out.push(digits[len] as char);
    }
}

fn push_u64(out: &mut String, mut n: u64) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut digits = [0u8; 24];
    let mut len = 0usize;
    while n > 0 {
        digits[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
    }
    while len > 0 {
        len -= 1;
        out.push(digits[len] as char);
    }
}

fn read_utf8_file(path: &[u8]) -> Result<String, &'static str> {
    let fd = libc::open(path).map_err(|_| "Could not open file")?;
    let mut out = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    loop {
        let n = libc::read(fd, &mut chunk).map_err(|_| "Read failed")?;
        if n == 0 {
            break;
        }
        if out.len() + n > MAX_FILE_BYTES {
            let _ = libc::close(fd);
            return Err("File too large");
        }
        out.extend_from_slice(&chunk[..n]);
    }
    let _ = libc::close(fd);
    core::str::from_utf8(&out)
        .map(String::from)
        .map_err(|_| "Invalid UTF-8")
}

fn write_utf8_file(path: &[u8], data: &[u8]) -> Result<(), &'static str> {
    let fd = libc::open_with_flags(path, libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC)
        .map_err(|_| "Could not open for writing")?;
    let mut offset = 0usize;
    while offset < data.len() {
        let n = libc::write(fd, &data[offset..]).map_err(|_| "Write failed")?;
        if n == 0 {
            let _ = libc::close(fd);
            return Err("Write stalled");
        }
        offset += n;
    }
    let _ = libc::close(fd);
    Ok(())
}

fn touch_empty_file(path: &[u8]) -> Result<(), &'static str> {
    write_utf8_file(path, &[])
}

fn user_home_dir() -> &'static str {
    if libc::getuid() == 0 {
        "/root"
    } else {
        "/home/user"
    }
}

fn create_temp_backing_path() -> Result<PathBuf, &'static str> {
    let pid = libc::getpid();
    let ticks = monotonic_millis();
    let mut candidates = [String::new(), String::new()];
    candidates[0].push_str("/tmp/sunedit-");
    push_u64(&mut candidates[0], pid);
    candidates[0].push('-');
    push_u64(&mut candidates[0], ticks);
    candidates[0].push_str(".txt");

    candidates[1].push_str("/root/sunedit-");
    push_u64(&mut candidates[1], pid);
    candidates[1].push('-');
    push_u64(&mut candidates[1], ticks);
    candidates[1].push_str(".txt");

    let _ = libc::mkdir_recursive(b"/tmp");
    for candidate in &candidates {
        let Some(path) = PathBuf::from_str(candidate) else {
            continue;
        };
        if touch_empty_file(path.as_bytes()).is_ok() {
            return Ok(path);
        }
    }
    Err("Could not create temporary file")
}

fn collect_argv(argc: u64, argv: *const *const u8) -> ([String; MAX_ARGC], usize) {
    let mut out = core::array::from_fn(|_| String::new());
    let mut raw = [core::ptr::null::<u8>(); MAX_ARGC];
    let count = unsafe { crt0::collect_raw_args(argc, argv, &mut raw) };
    let mut kept = 0usize;
    for i in 0..count.min(MAX_ARGC) {
        let len = unsafe { crt0::cstr_len(raw[i], PATH_LEN) };
        if len == 0 {
            continue;
        }
        let bytes = unsafe { core::slice::from_raw_parts(raw[i], len) };
        if let Ok(text) = core::str::from_utf8(bytes) {
            out[kept] = String::from(text);
            kept += 1;
        }
    }
    (out, kept)
}

fn parse_user_path_arg(argc: u64, argv: *const *const u8) -> Option<PathBuf> {
    let (args, count) = collect_argv(argc, argv);
    let slice: Vec<&str> = args[..count].iter().map(String::as_str).collect();
    let user_args = if count > 0 { &slice[1..] } else { &[][..] };
    extract_first_real_file_path(user_args).and_then(PathBuf::from_str)
}

fn path_parent_string(path: PathBuf) -> Option<String> {
    let text = path.as_str();
    let (parent, _) = text.rsplit_once('/')?;
    if parent.is_empty() {
        Some(String::from("/"))
    } else {
        Some(String::from(parent))
    }
}

fn show_dialog(request: &DialogRequest) -> Result<DialogResult, &'static str> {
    let cap = ensure_dialog_service().ok_or("Dialog host unavailable")?;
    let body = encode_request(request);
    let reply = call_dialog(cap, DialogMsg::SHOW_DIALOG, &body).map_err(dialog_error_message)?;
    if reply.label == DialogMsg::ERROR {
        return Err(dialog_error_message(DialogError::from_code(reply.words[0])));
    }
    let bytes = take_dialog_reply_bytes(&reply).map_err(dialog_error_message)?;
    decode_result(&bytes).map_err(dialog_error_message)
}

fn ensure_dialog_service() -> Option<CapabilityToken> {
    if let Some(cap) = nameserver_lookup("dialogd") {
        return Some(cap);
    }
    if let Some(cap) = nameserver_lookup_timeout("dialogd", 50) {
        return Some(cap);
    }
    let _ = libc::spawn(b"/sbin/sunlight-dialogd", &[b"sunlight-dialogd"], None)
        .or_else(|_| libc::spawn(b"/bin/sunlight-dialogd", &[b"sunlight-dialogd"], None));
    for _ in 0..8 {
        if let Some(cap) = nameserver_lookup_timeout("dialogd", 75) {
            return Some(cap);
        }
        process_yield();
    }
    None
}

fn call_dialog(cap: CapabilityToken, label: u64, body: &[u8]) -> Result<IpcMsg, DialogError> {
    if body.len() > SHM_PAGE {
        return Err(DialogError::TooLarge);
    }
    let (ptr, token) = shm_alloc().map_err(|_| DialogError::Internal)?;
    unsafe {
        core::ptr::copy_nonoverlapping(body.as_ptr(), ptr, body.len());
    }
    let reply = ipc_call(
        cap,
        IpcMsg::with_label(label)
            .word(0, body.len() as u64)
            .with_cap(0, token),
    );
    let _ = shm_free(token);
    Ok(reply)
}

fn take_dialog_reply_bytes(reply: &IpcMsg) -> Result<Vec<u8>, DialogError> {
    let len = reply.words[1] as usize;
    let token = reply.caps[0];
    if len == 0 || len > SHM_PAGE || token == CapabilityToken::INVALID {
        return Err(DialogError::Corrupt);
    }
    let ptr = shm_map(token).map_err(|_| DialogError::Corrupt)?;
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec();
    let _ = shm_free(token);
    Ok(bytes)
}

fn dialog_error_message(err: DialogError) -> &'static str {
    match err {
        DialogError::BadRequest => "Bad dialog request",
        DialogError::TooLarge => "Dialog payload too large",
        DialogError::Unsupported => "Dialog type not implemented",
        DialogError::Busy => "Dialog host is busy",
        DialogError::Internal => "Dialog failed",
        DialogError::HostUnavailable => "Dialog host unavailable",
        DialogError::Corrupt => "Dialog returned invalid data",
    }
}

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    sunlight_libc::launch_trace::init_from_argv(argc, argv);
    let trace = launch_trace::current().unwrap_or(LaunchTrace::new(0, LaunchSource::Unknown, 0));
    launch_trace::log_phase_now(
        trace,
        "app=sunlight-edit",
        "app_main_started",
        Some(sunlight_ipc::getpid()),
    );

    let pending_user_path = parse_user_path_arg(argc, argv);
    let mut app = EditApp::new(pending_user_path);

    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Sunlight Edit",
        decoration: WindowDecoration::Normal,
    }) {
        Some(w) => w,
        None => {
            debug_log("[EDIT] failed to connect window\n");
            loop {
                process_yield();
            }
        }
    };

    window.run(&mut app);
    ProcessExit::exit(0);
}

#[cfg(test)]
mod tests {
    use super::{EditApp, Rect, TextBuffer, WIN_H, WIN_W};
    use sunlight_ui::widgets::TextPosition;

    fn document(line_count: usize) -> TextBuffer {
        let mut text = String::new();
        for line in 0..line_count {
            if line > 0 {
                text.push('\n');
            }
            super::push_usize(&mut text, line);
        }
        TextBuffer::from_utf8(&text)
    }

    #[test]
    fn responsive_root_assigns_fill_width_and_remaining_editor_height() {
        let initial = EditApp::compute_layout(Rect::new(0, 0, WIN_W, WIN_H), false);
        let resized = EditApp::compute_layout(Rect::new(0, 0, 1120, 760), false);
        assert_eq!(initial.header.w, WIN_W);
        assert_eq!(resized.header.w, 1120);
        assert_eq!(resized.toolbar.w, 1120);
        assert_eq!(resized.editor.w, 1120);
        assert_eq!(resized.status.w, 1120);
        assert_eq!(resized.status.bottom(), resized.root.bottom());
        assert_eq!(
            resized.editor.h,
            760 - super::HEADER_H - super::TOOLBAR_H - super::STATUS_H
        );
    }

    #[test]
    fn find_panel_consumes_fixed_space_without_moving_status() {
        let root = Rect::new(0, 0, WIN_W, WIN_H);
        let hidden = EditApp::compute_layout(root, false);
        let shown = EditApp::compute_layout(root, true);
        assert_eq!(hidden.find.h, 0);
        assert_eq!(shown.find.h, super::FIND_PANEL_H);
        assert_eq!(shown.editor.h, hidden.editor.h - super::FIND_PANEL_H);
        assert_eq!(shown.status, hidden.status);
    }

    #[test]
    fn shrink_and_grow_update_live_editor_viewport() {
        let mut app = EditApp::new(None);
        let original = app.editor_rect();
        assert!(app.set_client_bounds(640, 360));
        let small = app.editor_rect();
        assert!(small.w < original.w && small.h < original.h);
        assert!(app.set_client_bounds(1180, 800));
        let large = app.editor_rect();
        assert!(large.w > original.w && large.h > original.h);
    }

    #[test]
    fn resize_clamps_scroll_without_resetting_valid_position() {
        let mut app = EditApp::new(None);
        app.buffer = document(80);
        app.buffer.cursor_line = 40;
        app.scroll_line = 35;
        app.selection.scroll_line = 35;
        assert!(app.set_client_bounds(700, 300));
        assert!(app.scroll_line > 0);
        let preserved = app.scroll_line;
        assert!(!app.set_client_bounds(700, 300));
        assert_eq!(app.scroll_line, preserved);

        app.buffer.cursor_line = 79;
        app.scroll_line = 79;
        assert!(app.set_client_bounds(1200, 900));
        let max_scroll = app
            .buffer
            .line_count()
            .saturating_sub(app.visible_line_count());
        assert_eq!(app.scroll_line, max_scroll);
    }

    #[test]
    fn resize_preserves_logical_caret_and_selection() {
        let mut app = EditApp::new(None);
        app.buffer = document(30);
        app.buffer.cursor_line = 18;
        app.buffer.cursor_col = 1;
        app.selection.anchor = Some(TextPosition { line: 2, col: 0 });
        let caret = app.buffer.cursor();
        let anchor = app.selection.anchor;
        app.set_client_bounds(520, 220);
        assert_eq!(app.buffer.cursor(), caret);
        assert_eq!(app.selection.anchor, anchor);
        assert!(app.buffer.cursor_line >= app.scroll_line);
        assert!(app.buffer.cursor_line < app.scroll_line + app.visible_line_count());
    }

    #[test]
    fn zero_and_tiny_roots_produce_zero_content_viewports_safely() {
        for bounds in [Rect::new(0, 0, 0, 0), Rect::new(0, 0, 1, 1)] {
            let layout = EditApp::compute_layout(bounds, true);
            assert_eq!(layout.root, bounds);
            assert_eq!(layout.editor.w, bounds.w);
            assert_eq!(layout.editor.h, 0);
        }
    }

    #[test]
    fn grow_shrink_grow_layout_is_deterministic() {
        let large = Rect::new(0, 0, 1100, 760);
        let first = EditApp::compute_layout(large, false);
        let _ = EditApp::compute_layout(Rect::new(0, 0, 430, 180), false);
        assert_eq!(first, EditApp::compute_layout(large, false));
    }
}
