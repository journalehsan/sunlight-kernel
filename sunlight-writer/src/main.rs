#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use core::alloc::GlobalAlloc;

use sun_font::{draw_text, draw_text_vcenter, measure_text, FontRole, TextStyle};
use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    process_yield, ProcessExit,
};
use sunlight_ui::image::TgaImage;
use sunlight_ui::widgets::StatusBar;
use sunlight_ui::{
    request_close, App, Canvas, Color, Event, Point, Rect, Theme, Window, WindowConfig,
    WindowDecoration,
};

const WIN_W: u32 = 1240;
const WIN_H: u32 = 860;
const TOP_BAR_H: u32 = 52;
const RIBBON_H: u32 = 122;
const STATUS_H: u32 = 22;
const APP_MENU_X: i32 = 14;
const APP_MENU_Y_GAP: i32 = 6;
const APP_MENU_LEFT_W: u32 = 222;
const APP_MENU_RIGHT_W: u32 = 300;
const APP_MENU_HEADER_H: u32 = 34;
const APP_MENU_ITEM_H: u32 = 30;
const APP_MENU_OPEN_INDEX: usize = 1;
const MENU_BUTTON_W: u32 = 44;
const QUICK_CHIP_H: u32 = 28;
const CONTENT_PAD: i32 = 18;
const MSG_LEN: usize = 96;
const KEY_ESC: u8 = 0x01;

static ICON_MENU_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_menu.tga"));
static ICON_NEW_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_new.tga"));
static ICON_OPEN_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_open.tga"));
static ICON_SAVE_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_save.tga"));
static ICON_PRINT_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_print.tga"));
static ICON_SHARE_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_share.tga"));
static ICON_DOC_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_doc.tga"));
static ICON_BOLD_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_bold.tga"));
static ICON_ITALIC_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_italic.tga"));
static ICON_UNDERLINE_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_underline.tga"));
static ICON_ALIGN_LEFT_TGA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/icon_align_left.tga"));
static ICON_ALIGN_CENTER_TGA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/icon_align_center.tga"));
static ICON_ALIGN_RIGHT_TGA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/icon_align_right.tga"));
static ICON_ALIGN_JUSTIFY_TGA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/icon_align_justify.tga"));
static ICON_BULLETS_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_bullets.tga"));
static ICON_NUMBERING_TGA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/icon_numbering.tga"));
static ICON_PICTURE_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_picture.tga"));
static ICON_LINK_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_link.tga"));

struct BumpAllocator;
unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        const HEAP_SIZE: usize = 1024 * 1024;
        static mut HEAP: [u8; 1024 * 1024] = [0; 1024 * 1024];
        static mut NEXT: usize = 0;
        let aligned = (NEXT + layout.align() - 1) & !(layout.align() - 1);
        let end = aligned + layout.size();
        if end > HEAP_SIZE {
            return core::ptr::null_mut();
        }
        NEXT = end;
        core::ptr::addr_of_mut!(HEAP).cast::<u8>().add(aligned)
    }

    unsafe fn dealloc(&self, _: *mut u8, _: core::alloc::Layout) {}
}

#[global_allocator]
static ALLOC: BumpAllocator = BumpAllocator;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[WRITER] panic\n");
    loop {
        process_yield();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IconId {
    Menu,
    New,
    Open,
    Save,
    Print,
    Share,
    Doc,
    Bold,
    Italic,
    Underline,
    AlignLeft,
    AlignCenter,
    AlignRight,
    AlignJustify,
    Bullets,
    Numbering,
    Picture,
    Link,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WriterAction {
    New,
    Open,
    Save,
    SaveAs,
    Print,
    Share,
    Export,
    Exit,
    FontFamily,
    FontSize,
    Bold,
    Italic,
    Underline,
    AlignLeft,
    AlignCenter,
    AlignRight,
    AlignJustify,
    Bullets,
    Numbering,
    InsertPicture,
    InsertShape,
    InsertTable,
    InsertLink,
    RecentDocument(usize),
}

#[derive(Clone, Copy)]
struct AppMenuItem {
    label: &'static str,
    action: WriterAction,
    icon: Option<IconId>,
    submenu: bool,
}

#[derive(Clone, Copy)]
struct RecentDocument {
    title: &'static str,
    meta: &'static str,
}

#[derive(Clone, Copy)]
enum RibbonControlKind {
    Dropdown,
    Toggle,
    IconButton,
    WideButton,
}

#[derive(Clone, Copy)]
struct RibbonControl {
    label: &'static str,
    icon: Option<IconId>,
    width: u32,
    kind: RibbonControlKind,
    action: WriterAction,
}

#[derive(Clone, Copy)]
struct RibbonGroup {
    title: &'static str,
    controls: &'static [RibbonControl],
}

#[derive(Clone, Copy)]
struct QuickChip {
    label: &'static str,
    icon: Option<IconId>,
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

    fn set(&mut self, text: &str) {
        let bytes = text.as_bytes();
        self.len = bytes.len().min(MSG_LEN);
        self.buf[..self.len].copy_from_slice(&bytes[..self.len]);
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    fn as_str(&self) -> &str {
        if self.len == 0 {
            return "";
        }
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

struct WriterIcons {
    menu: Option<TgaImage>,
    new_doc: Option<TgaImage>,
    open: Option<TgaImage>,
    save: Option<TgaImage>,
    print: Option<TgaImage>,
    share: Option<TgaImage>,
    doc: Option<TgaImage>,
    bold: Option<TgaImage>,
    italic: Option<TgaImage>,
    underline: Option<TgaImage>,
    align_left: Option<TgaImage>,
    align_center: Option<TgaImage>,
    align_right: Option<TgaImage>,
    align_justify: Option<TgaImage>,
    bullets: Option<TgaImage>,
    numbering: Option<TgaImage>,
    picture: Option<TgaImage>,
    link: Option<TgaImage>,
}

impl WriterIcons {
    fn load() -> Self {
        Self {
            menu: TgaImage::parse(ICON_MENU_TGA).ok(),
            new_doc: TgaImage::parse(ICON_NEW_TGA).ok(),
            open: TgaImage::parse(ICON_OPEN_TGA).ok(),
            save: TgaImage::parse(ICON_SAVE_TGA).ok(),
            print: TgaImage::parse(ICON_PRINT_TGA).ok(),
            share: TgaImage::parse(ICON_SHARE_TGA).ok(),
            doc: TgaImage::parse(ICON_DOC_TGA).ok(),
            bold: TgaImage::parse(ICON_BOLD_TGA).ok(),
            italic: TgaImage::parse(ICON_ITALIC_TGA).ok(),
            underline: TgaImage::parse(ICON_UNDERLINE_TGA).ok(),
            align_left: TgaImage::parse(ICON_ALIGN_LEFT_TGA).ok(),
            align_center: TgaImage::parse(ICON_ALIGN_CENTER_TGA).ok(),
            align_right: TgaImage::parse(ICON_ALIGN_RIGHT_TGA).ok(),
            align_justify: TgaImage::parse(ICON_ALIGN_JUSTIFY_TGA).ok(),
            bullets: TgaImage::parse(ICON_BULLETS_TGA).ok(),
            numbering: TgaImage::parse(ICON_NUMBERING_TGA).ok(),
            picture: TgaImage::parse(ICON_PICTURE_TGA).ok(),
            link: TgaImage::parse(ICON_LINK_TGA).ok(),
        }
    }

    fn get(&self, icon: IconId) -> Option<&TgaImage> {
        match icon {
            IconId::Menu => self.menu.as_ref(),
            IconId::New => self.new_doc.as_ref(),
            IconId::Open => self.open.as_ref(),
            IconId::Save => self.save.as_ref(),
            IconId::Print => self.print.as_ref(),
            IconId::Share => self.share.as_ref(),
            IconId::Doc => self.doc.as_ref(),
            IconId::Bold => self.bold.as_ref(),
            IconId::Italic => self.italic.as_ref(),
            IconId::Underline => self.underline.as_ref(),
            IconId::AlignLeft => self.align_left.as_ref(),
            IconId::AlignCenter => self.align_center.as_ref(),
            IconId::AlignRight => self.align_right.as_ref(),
            IconId::AlignJustify => self.align_justify.as_ref(),
            IconId::Bullets => self.bullets.as_ref(),
            IconId::Numbering => self.numbering.as_ref(),
            IconId::Picture => self.picture.as_ref(),
            IconId::Link => self.link.as_ref(),
        }
    }
}

const APP_MENU_ITEMS: [AppMenuItem; 8] = [
    AppMenuItem {
        label: "New",
        action: WriterAction::New,
        icon: Some(IconId::New),
        submenu: false,
    },
    AppMenuItem {
        label: "Open",
        action: WriterAction::Open,
        icon: Some(IconId::Open),
        submenu: true,
    },
    AppMenuItem {
        label: "Save",
        action: WriterAction::Save,
        icon: Some(IconId::Save),
        submenu: false,
    },
    AppMenuItem {
        label: "Save As",
        action: WriterAction::SaveAs,
        icon: Some(IconId::Save),
        submenu: false,
    },
    AppMenuItem {
        label: "Print",
        action: WriterAction::Print,
        icon: Some(IconId::Print),
        submenu: false,
    },
    AppMenuItem {
        label: "Share",
        action: WriterAction::Share,
        icon: Some(IconId::Share),
        submenu: false,
    },
    AppMenuItem {
        label: "Export",
        action: WriterAction::Export,
        icon: Some(IconId::Share),
        submenu: false,
    },
    AppMenuItem {
        label: "Exit",
        action: WriterAction::Exit,
        icon: None,
        submenu: false,
    },
];

const RECENT_DOCS: [RecentDocument; 4] = [
    RecentDocument {
        title: "Board Meeting Memo.swdoc",
        meta: "Today . 18 KB",
    },
    RecentDocument {
        title: "Quarterly Narrative Draft.swdoc",
        meta: "Yesterday . 124 KB",
    },
    RecentDocument {
        title: "SunlightOS Launch Notes.swdoc",
        meta: "July 7 . 36 KB",
    },
    RecentDocument {
        title: "Partner Briefing Outline.swdoc",
        meta: "July 4 . 42 KB",
    },
];

const QUICK_CHIPS: [QuickChip; 3] = [
    QuickChip {
        label: "Secure Draft",
        icon: Some(IconId::Doc),
    },
    QuickChip {
        label: "Premium Workspace",
        icon: None,
    },
    QuickChip {
        label: "Canvas Pending",
        icon: None,
    },
];

const FILE_GROUP_CONTROLS: [RibbonControl; 4] = [
    RibbonControl {
        label: "New",
        icon: Some(IconId::New),
        width: 58,
        kind: RibbonControlKind::WideButton,
        action: WriterAction::New,
    },
    RibbonControl {
        label: "Open",
        icon: Some(IconId::Open),
        width: 58,
        kind: RibbonControlKind::WideButton,
        action: WriterAction::Open,
    },
    RibbonControl {
        label: "Save",
        icon: Some(IconId::Save),
        width: 58,
        kind: RibbonControlKind::WideButton,
        action: WriterAction::Save,
    },
    RibbonControl {
        label: "Print",
        icon: Some(IconId::Print),
        width: 58,
        kind: RibbonControlKind::WideButton,
        action: WriterAction::Print,
    },
];

const FONT_GROUP_CONTROLS: [RibbonControl; 5] = [
    RibbonControl {
        label: "Inter",
        icon: None,
        width: 120,
        kind: RibbonControlKind::Dropdown,
        action: WriterAction::FontFamily,
    },
    RibbonControl {
        label: "12",
        icon: None,
        width: 58,
        kind: RibbonControlKind::Dropdown,
        action: WriterAction::FontSize,
    },
    RibbonControl {
        label: "B",
        icon: Some(IconId::Bold),
        width: 36,
        kind: RibbonControlKind::Toggle,
        action: WriterAction::Bold,
    },
    RibbonControl {
        label: "I",
        icon: Some(IconId::Italic),
        width: 36,
        kind: RibbonControlKind::Toggle,
        action: WriterAction::Italic,
    },
    RibbonControl {
        label: "U",
        icon: Some(IconId::Underline),
        width: 36,
        kind: RibbonControlKind::Toggle,
        action: WriterAction::Underline,
    },
];

const PARAGRAPH_GROUP_CONTROLS: [RibbonControl; 6] = [
    RibbonControl {
        label: "",
        icon: Some(IconId::AlignLeft),
        width: 36,
        kind: RibbonControlKind::IconButton,
        action: WriterAction::AlignLeft,
    },
    RibbonControl {
        label: "",
        icon: Some(IconId::AlignCenter),
        width: 36,
        kind: RibbonControlKind::IconButton,
        action: WriterAction::AlignCenter,
    },
    RibbonControl {
        label: "",
        icon: Some(IconId::AlignRight),
        width: 36,
        kind: RibbonControlKind::IconButton,
        action: WriterAction::AlignRight,
    },
    RibbonControl {
        label: "",
        icon: Some(IconId::AlignJustify),
        width: 36,
        kind: RibbonControlKind::IconButton,
        action: WriterAction::AlignJustify,
    },
    RibbonControl {
        label: "",
        icon: Some(IconId::Bullets),
        width: 36,
        kind: RibbonControlKind::IconButton,
        action: WriterAction::Bullets,
    },
    RibbonControl {
        label: "",
        icon: Some(IconId::Numbering),
        width: 36,
        kind: RibbonControlKind::IconButton,
        action: WriterAction::Numbering,
    },
];

const INSERT_GROUP_CONTROLS: [RibbonControl; 4] = [
    RibbonControl {
        label: "Picture",
        icon: Some(IconId::Picture),
        width: 80,
        kind: RibbonControlKind::WideButton,
        action: WriterAction::InsertPicture,
    },
    RibbonControl {
        label: "Shape",
        icon: None,
        width: 70,
        kind: RibbonControlKind::WideButton,
        action: WriterAction::InsertShape,
    },
    RibbonControl {
        label: "Table",
        icon: None,
        width: 70,
        kind: RibbonControlKind::WideButton,
        action: WriterAction::InsertTable,
    },
    RibbonControl {
        label: "Link",
        icon: Some(IconId::Link),
        width: 66,
        kind: RibbonControlKind::WideButton,
        action: WriterAction::InsertLink,
    },
];

const RIBBON_GROUPS: [RibbonGroup; 4] = [
    RibbonGroup {
        title: "File",
        controls: &FILE_GROUP_CONTROLS,
    },
    RibbonGroup {
        title: "Font",
        controls: &FONT_GROUP_CONTROLS,
    },
    RibbonGroup {
        title: "Paragraph",
        controls: &PARAGRAPH_GROUP_CONTROLS,
    },
    RibbonGroup {
        title: "Insert",
        controls: &INSERT_GROUP_CONTROLS,
    },
];

struct WriterApp {
    icons: WriterIcons,
    menu_open: bool,
    menu_hover: Option<usize>,
    menu_pinned: Option<usize>,
    recent_hover: Option<usize>,
    quick_hover: Option<usize>,
    ribbon_hover: Option<(usize, usize)>,
    menu_button_hover: bool,
    status_center: TextSlot,
    status_ticks: u16,
}

impl WriterApp {
    fn new() -> Self {
        let mut status_center = TextSlot::empty();
        status_center.set("Canvas Widget Placeholder");
        Self {
            icons: WriterIcons::load(),
            menu_open: false,
            menu_hover: None,
            menu_pinned: None,
            recent_hover: None,
            quick_hover: None,
            ribbon_hover: None,
            menu_button_hover: false,
            status_center,
            status_ticks: 0,
        }
    }

    fn set_status_message(&mut self, text: &str) {
        self.status_center.set(text);
        self.status_ticks = 32;
    }

    fn top_bar_rect(&self) -> Rect {
        Rect::new(0, 0, WIN_W, TOP_BAR_H)
    }

    fn ribbon_rect(&self) -> Rect {
        Rect::new(0, TOP_BAR_H as i32, WIN_W, RIBBON_H)
    }

    fn content_rect(&self) -> Rect {
        let top = TOP_BAR_H + RIBBON_H;
        Rect::new(
            0,
            top as i32,
            WIN_W,
            WIN_H.saturating_sub(top + STATUS_H),
        )
    }

    fn status_rect(&self) -> Rect {
        Rect::new(0, (WIN_H - STATUS_H) as i32, WIN_W, STATUS_H)
    }

    fn menu_button_rect(&self) -> Rect {
        Rect::new(APP_MENU_X, 10, MENU_BUTTON_W, 32)
    }

    fn command_title_rect(&self) -> Rect {
        Rect::new(APP_MENU_X + MENU_BUTTON_W as i32 + 12, 7, 420, 18)
    }

    fn command_subtitle_rect(&self) -> Rect {
        Rect::new(APP_MENU_X + MENU_BUTTON_W as i32 + 12, 26, 420, 16)
    }

    fn quick_chip_rect(&self, idx: usize) -> Rect {
        let base_y = 12;
        let widths = [104u32, 144u32, 132u32];
        let mut x = WIN_W as i32 - 24;
        for width in widths[..=idx].iter().rev() {
            x -= *width as i32;
            if idx != 0 || *width != widths[idx] {
                x -= 8;
            }
        }
        Rect::new(x, base_y, widths[idx], QUICK_CHIP_H)
    }

    fn active_menu_index(&self) -> Option<usize> {
        self.menu_hover.or(self.menu_pinned)
    }

    fn submenu_visible(&self) -> bool {
        self.active_menu_index() == Some(APP_MENU_OPEN_INDEX)
    }

    fn app_menu_rect(&self) -> Rect {
        let width = if self.submenu_visible() {
            APP_MENU_LEFT_W + APP_MENU_RIGHT_W
        } else {
            APP_MENU_LEFT_W
        };
        let height =
            APP_MENU_HEADER_H + APP_MENU_ITEMS.len() as u32 * APP_MENU_ITEM_H + 12;
        Rect::new(
            APP_MENU_X,
            self.top_bar_rect().bottom() + APP_MENU_Y_GAP,
            width,
            height,
        )
    }

    fn app_menu_left_rect(&self) -> Rect {
        let panel = self.app_menu_rect();
        Rect::new(panel.x, panel.y, APP_MENU_LEFT_W, panel.h)
    }

    fn app_menu_right_rect(&self) -> Rect {
        let left = self.app_menu_left_rect();
        Rect::new(left.right(), left.y, APP_MENU_RIGHT_W, left.h)
    }

    fn menu_item_rect(&self, idx: usize) -> Rect {
        let left = self.app_menu_left_rect();
        Rect::new(
            left.x + 8,
            left.y + APP_MENU_HEADER_H as i32 + 4 + idx as i32 * APP_MENU_ITEM_H as i32,
            APP_MENU_LEFT_W - 16,
            APP_MENU_ITEM_H,
        )
    }

    fn recent_doc_rect(&self, idx: usize) -> Rect {
        let right = self.app_menu_right_rect();
        Rect::new(
            right.x + 10,
            right.y + APP_MENU_HEADER_H as i32 + 4 + idx as i32 * 42,
            APP_MENU_RIGHT_W - 20,
            38,
        )
    }

    fn document_host_rect(&self) -> Rect {
        self.content_rect().inset(CONTENT_PAD)
    }

    fn document_page_rect(&self) -> Rect {
        let host = self.document_host_rect();
        let desired_w = 860u32.min(host.w.saturating_sub(96)).max(620);
        let desired_h = host.h.saturating_sub(56).max(420);
        let x = host.x + ((host.w as i32 - desired_w as i32) / 2);
        let y = host.y + 26;
        Rect::new(x, y, desired_w, desired_h)
    }

    fn canvas_insertion_rect(&self) -> Rect {
        self.document_page_rect().inset(24)
    }

    fn ribbon_group_rects(&self) -> [Rect; 4] {
        let ribbon = self.ribbon_rect();
        let widths = [258u32, 292u32, 264u32, 302u32];
        let mut rects = [Rect::new(0, 0, 0, 0); 4];
        let mut x = 16;
        for (idx, width) in widths.iter().enumerate() {
            rects[idx] = Rect::new(x, ribbon.y + 12, *width, ribbon.h - 20);
            x += *width as i32 + 10;
        }
        rects
    }

    fn ribbon_control_rect(&self, group_idx: usize, control_idx: usize) -> Rect {
        let group = self.ribbon_group_rects()[group_idx];
        let control = RIBBON_GROUPS[group_idx].controls[control_idx];
        let mut x = group.x + 12;
        for prev in &RIBBON_GROUPS[group_idx].controls[..control_idx] {
            x += prev.width as i32 + 8;
        }
        let y = group.y + 12;
        Rect::new(x, y, control.width, 36)
    }

    fn quick_chip_hit(&self, point: Point) -> Option<usize> {
        QUICK_CHIPS
            .iter()
            .enumerate()
            .find_map(|(idx, _)| self.quick_chip_rect(idx).contains(point).then_some(idx))
    }

    fn menu_item_hit(&self, point: Point) -> Option<usize> {
        APP_MENU_ITEMS
            .iter()
            .enumerate()
            .find_map(|(idx, _)| self.menu_item_rect(idx).contains(point).then_some(idx))
    }

    fn recent_doc_hit(&self, point: Point) -> Option<usize> {
        if !self.submenu_visible() {
            return None;
        }
        RECENT_DOCS
            .iter()
            .enumerate()
            .find_map(|(idx, _)| self.recent_doc_rect(idx).contains(point).then_some(idx))
    }

    fn ribbon_hit(&self, point: Point) -> Option<(usize, usize)> {
        for (group_idx, group) in RIBBON_GROUPS.iter().enumerate() {
            for (control_idx, _) in group.controls.iter().enumerate() {
                if self.ribbon_control_rect(group_idx, control_idx).contains(point) {
                    return Some((group_idx, control_idx));
                }
            }
        }
        None
    }

    fn toggle_menu(&mut self) {
        self.menu_open = !self.menu_open;
        if self.menu_open {
            self.menu_hover = None;
            self.menu_pinned = None;
            self.recent_hover = None;
            self.set_status_message("Application menu opened");
        } else {
            self.menu_hover = None;
            self.menu_pinned = None;
            self.recent_hover = None;
            self.set_status_message("Application menu closed");
        }
    }

    fn close_menu(&mut self) {
        self.menu_open = false;
        self.menu_hover = None;
        self.menu_pinned = None;
        self.recent_hover = None;
    }

    fn dispatch_action(&mut self, action: WriterAction) -> bool {
        match action {
            WriterAction::New => self.set_status_message("New document is a UI placeholder"),
            WriterAction::Open => self.set_status_message("Open panel is a UI placeholder"),
            WriterAction::Save => self.set_status_message("Save is not implemented in this phase"),
            WriterAction::SaveAs => {
                self.set_status_message("Save As is not implemented in this phase")
            }
            WriterAction::Print => self.set_status_message("Print is a placeholder command"),
            WriterAction::Share => self.set_status_message("Share is a placeholder command"),
            WriterAction::Export => self.set_status_message("Export is a placeholder command"),
            WriterAction::Exit => {
                request_close();
                return false;
            }
            WriterAction::FontFamily => self.set_status_message("Font picker is visual only"),
            WriterAction::FontSize => self.set_status_message("Font size picker is visual only"),
            WriterAction::Bold => self.set_status_message("Bold toggle placeholder"),
            WriterAction::Italic => self.set_status_message("Italic toggle placeholder"),
            WriterAction::Underline => self.set_status_message("Underline toggle placeholder"),
            WriterAction::AlignLeft => self.set_status_message("Align Left placeholder"),
            WriterAction::AlignCenter => self.set_status_message("Align Center placeholder"),
            WriterAction::AlignRight => self.set_status_message("Align Right placeholder"),
            WriterAction::AlignJustify => self.set_status_message("Justify placeholder"),
            WriterAction::Bullets => self.set_status_message("Bullets placeholder"),
            WriterAction::Numbering => self.set_status_message("Numbering placeholder"),
            WriterAction::InsertPicture => self.set_status_message("Insert Picture placeholder"),
            WriterAction::InsertShape => self.set_status_message("Insert Shape placeholder"),
            WriterAction::InsertTable => self.set_status_message("Insert Table placeholder"),
            WriterAction::InsertLink => self.set_status_message("Insert Link placeholder"),
            WriterAction::RecentDocument(idx) => {
                let doc = RECENT_DOCS[idx];
                let mut msg = String::from("Recent document preview: ");
                msg.push_str(doc.title);
                self.set_status_message(&msg);
            }
        }
        true
    }

    fn draw_top_bar(&self, canvas: &mut Canvas, theme: &Theme) {
        let rect = self.top_bar_rect();
        fill_vertical_gradient(
            canvas,
            rect,
            theme.panel.lighten(10),
            theme.panel.darken(24),
        );
        canvas.hbar(rect.x, rect.bottom() - 1, rect.w, 1, theme.border);
        canvas.hbar(rect.x, rect.bottom() - 2, rect.w, 1, theme.accent.darken(110));

        let menu_button = self.menu_button_rect();
        let menu_fill = if self.menu_open {
            theme.accent.darken(42)
        } else if self.menu_button_hover {
            theme.panel_alt.lighten(18)
        } else {
            theme.panel_alt
        };
        canvas.fill_rounded_rect(menu_button, 8, menu_fill);
        canvas.stroke_rounded_rect(
            menu_button,
            8,
            1,
            if self.menu_open {
                theme.accent
            } else {
                theme.border
            },
        );
        if let Some(icon) = self.icons.get(IconId::Menu) {
            canvas.draw_tga_icon_tinted(
                icon,
                Rect::new(menu_button.x + 10, menu_button.y + 8, 20, 20),
                if self.menu_open {
                    theme.text_on_accent
                } else {
                    theme.icon_foreground
                },
            );
        }

        draw_text(
            canvas,
            "Sunlight Writer",
            self.command_title_rect().x,
            self.command_title_rect().y,
            &TextStyle::new(FontRole::UiTitle, theme.text),
        );
        draw_text(
            canvas,
            "Professional document shell . ribbon workspace . canvas-ready layout",
            self.command_subtitle_rect().x,
            self.command_subtitle_rect().y,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );

        for (idx, chip) in QUICK_CHIPS.iter().enumerate() {
            let rect = self.quick_chip_rect(idx);
            let hovered = self.quick_hover == Some(idx);
            canvas.fill_rounded_rect(
                rect,
                12,
                if hovered {
                    theme.panel_alt.lighten(12)
                } else {
                    theme.panel
                },
            );
            canvas.stroke_rounded_rect(
                rect,
                12,
                1,
                if idx == 1 {
                    theme.accent.darken(90)
                } else {
                    theme.border
                },
            );
            if let Some(icon_id) = chip.icon {
                if let Some(icon) = self.icons.get(icon_id) {
                    canvas.draw_tga_icon_tinted(
                        icon,
                        Rect::new(rect.x + 8, rect.y + 6, 16, 16),
                        if idx == 0 { theme.accent } else { theme.icon_muted },
                    );
                }
            }
            let text_x = if chip.icon.is_some() { rect.x + 28 } else { rect.x + 12 };
            draw_text_vcenter(
                canvas,
                chip.label,
                text_x,
                rect.y,
                rect.h,
                &TextStyle::new(
                    FontRole::UiSmall,
                    if idx == 1 { theme.text } else { theme.text_muted },
                ),
            );
        }
    }

    fn draw_ribbon(&self, canvas: &mut Canvas, theme: &Theme) {
        let rect = self.ribbon_rect();
        fill_vertical_gradient(
            canvas,
            rect,
            theme.panel_alt.lighten(8),
            theme.panel.darken(12),
        );
        canvas.hbar(rect.x, rect.bottom() - 1, rect.w, 1, theme.border);

        for (group_idx, group) in RIBBON_GROUPS.iter().enumerate() {
            let group_rect = self.ribbon_group_rects()[group_idx];
            canvas.fill_rounded_rect(group_rect, 12, theme.panel.lighten(4));
            canvas.stroke_rounded_rect(group_rect, 12, 1, theme.border);
            canvas.hbar(
                group_rect.x + 10,
                group_rect.bottom() - 24,
                group_rect.w - 20,
                1,
                theme.border,
            );

            for (control_idx, control) in group.controls.iter().enumerate() {
                self.draw_ribbon_control(
                    canvas,
                    theme,
                    group_idx,
                    control_idx,
                    *control,
                    self.ribbon_hover == Some((group_idx, control_idx)),
                );
            }

            draw_text_vcenter(
                canvas,
                group.title,
                group_rect.x + 12,
                group_rect.bottom() - 22,
                18,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
        }
    }

    fn draw_ribbon_control(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        group_idx: usize,
        control_idx: usize,
        control: RibbonControl,
        hovered: bool,
    ) {
        let rect = self.ribbon_control_rect(group_idx, control_idx);
        let fill = match control.kind {
            RibbonControlKind::Dropdown => theme.panel_alt.lighten(6),
            RibbonControlKind::Toggle => {
                if hovered {
                    theme.accent.darken(36)
                } else {
                    theme.panel_alt
                }
            }
            RibbonControlKind::IconButton | RibbonControlKind::WideButton => {
                if hovered {
                    theme.panel_alt.lighten(18)
                } else {
                    theme.panel_alt
                }
            }
        };
        let border = if hovered {
            theme.accent.darken(70)
        } else {
            theme.border
        };

        canvas.fill_rounded_rect(rect, 8, fill);
        canvas.stroke_rounded_rect(rect, 8, 1, border);

        match control.kind {
            RibbonControlKind::Dropdown => {
                draw_text_vcenter(
                    canvas,
                    control.label,
                    rect.x + 10,
                    rect.y,
                    rect.h,
                    &TextStyle::new(FontRole::UiRegular, theme.text),
                );
                let arrow_x = rect.right() - 18;
                let cy = rect.y + rect.h as i32 / 2;
                canvas.put_pixel(arrow_x, cy - 2, theme.text_dim);
                canvas.put_pixel(arrow_x + 1, cy - 1, theme.text_dim);
                canvas.put_pixel(arrow_x + 2, cy, theme.text_dim);
                canvas.put_pixel(arrow_x + 3, cy - 1, theme.text_dim);
                canvas.put_pixel(arrow_x + 4, cy - 2, theme.text_dim);
            }
            RibbonControlKind::Toggle | RibbonControlKind::IconButton => {
                if let Some(icon_id) = control.icon {
                    if let Some(icon) = self.icons.get(icon_id) {
                        canvas.draw_tga_icon_tinted(
                            icon,
                            Rect::new(rect.x + 8, rect.y + 8, 20, 20),
                            if hovered {
                                theme.accent
                            } else {
                                theme.icon_foreground
                            },
                        );
                    }
                }
                if matches!(control.kind, RibbonControlKind::Toggle) {
                    draw_text_vcenter(
                        canvas,
                        control.label,
                        rect.x + 14,
                        rect.y,
                        rect.h,
                        &TextStyle::new(FontRole::UiMedium, theme.text),
                    );
                }
            }
            RibbonControlKind::WideButton => {
                if let Some(icon_id) = control.icon {
                    if let Some(icon) = self.icons.get(icon_id) {
                        canvas.draw_tga_icon_tinted(
                            icon,
                            Rect::new(rect.x + 8, rect.y + 8, 18, 18),
                            if hovered {
                                theme.accent
                            } else {
                                theme.icon_foreground
                            },
                        );
                    }
                }
                draw_text_vcenter(
                    canvas,
                    control.label,
                    rect.x + if control.icon.is_some() { 30 } else { 10 },
                    rect.y,
                    rect.h,
                    &TextStyle::new(FontRole::UiRegular, theme.text),
                );
            }
        }
    }

    fn draw_app_menu(&self, canvas: &mut Canvas, theme: &Theme) {
        if !self.menu_open {
            return;
        }

        let panel = self.app_menu_rect();
        let left = self.app_menu_left_rect();
        canvas.fill_rounded_rect(panel, 14, theme.panel.lighten(4));
        canvas.stroke_rounded_rect(panel, 14, 1, theme.border);
        canvas.fill_rect(Rect::new(left.x, left.y, left.w, left.h), theme.panel.lighten(4));

        draw_text(
            canvas,
            "Application Menu",
            left.x + 12,
            left.y + 10,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );
        draw_text(
            canvas,
            "Writer shell commands",
            left.x + 12,
            left.y + 22,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        canvas.fill_rect(Rect::new(left.x + 8, left.y + 8, 4, 18), theme.accent);

        for (idx, item) in APP_MENU_ITEMS.iter().enumerate() {
            let rect = self.menu_item_rect(idx);
            let active = self.active_menu_index() == Some(idx);
            canvas.fill_rounded_rect(
                rect,
                8,
                if active {
                    theme.accent.darken(34)
                } else if idx % 2 == 0 {
                    theme.panel_alt
                } else {
                    theme.panel
                },
            );
            if active {
                canvas.stroke_rounded_rect(rect, 8, 1, theme.accent);
            }
            if let Some(icon_id) = item.icon {
                if let Some(icon) = self.icons.get(icon_id) {
                    canvas.draw_tga_icon_tinted(
                        icon,
                        Rect::new(rect.x + 8, rect.y + 6, 18, 18),
                        if active {
                            theme.accent_hover
                        } else {
                            theme.icon_foreground
                        },
                    );
                }
            }
            draw_text_vcenter(
                canvas,
                item.label,
                rect.x + 34,
                rect.y,
                rect.h,
                &TextStyle::new(
                    FontRole::UiRegular,
                    if active { theme.text } else { theme.text_muted },
                ),
            );
            if item.submenu {
                draw_text_vcenter(
                    canvas,
                    ">",
                    rect.right() - 18,
                    rect.y,
                    rect.h,
                    &TextStyle::new(FontRole::UiRegular, theme.text_dim),
                );
            }
        }

        if !self.submenu_visible() {
            return;
        }

        let right = self.app_menu_right_rect();
        canvas.fill_rect(right, theme.panel_alt.lighten(6));
        canvas.vline(right.x, right.y + 10, right.h - 20, theme.border);
        draw_text(
            canvas,
            "Recent Documents",
            right.x + 16,
            right.y + 10,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );
        draw_text(
            canvas,
            "Open continues into this column",
            right.x + 16,
            right.y + 22,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );

        for (idx, doc) in RECENT_DOCS.iter().enumerate() {
            let rect = self.recent_doc_rect(idx);
            let hovered = self.recent_hover == Some(idx);
            canvas.fill_rounded_rect(
                rect,
                8,
                if hovered {
                    theme.panel.lighten(10)
                } else {
                    theme.panel
                },
            );
            canvas.stroke_rounded_rect(
                rect,
                8,
                1,
                if hovered {
                    theme.accent.darken(70)
                } else {
                    theme.border
                },
            );
            if let Some(icon) = self.icons.get(IconId::Doc) {
                canvas.draw_tga_icon_tinted(
                    icon,
                    Rect::new(rect.x + 8, rect.y + 10, 16, 16),
                    if hovered { theme.accent } else { theme.icon_muted },
                );
            }
            draw_text(
                canvas,
                doc.title,
                rect.x + 30,
                rect.y + 8,
                &TextStyle::new(FontRole::UiRegular, theme.text),
            );
            draw_text(
                canvas,
                doc.meta,
                rect.x + 30,
                rect.y + 22,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
        }
    }

    fn draw_canvas_placeholder(&self, canvas: &mut Canvas, theme: &Theme) {
        let content = self.content_rect();
        fill_vertical_gradient(
            canvas,
            content,
            theme.bg.lighten(14),
            theme.bg.darken(8),
        );

        let host = self.document_host_rect();
        canvas.fill_rounded_rect(host, 20, theme.panel.darken(8));
        canvas.stroke_rounded_rect(host, 20, 1, theme.border);

        let page = self.document_page_rect();
        let shadow = page.translate(10, 12);
        canvas.fill_rounded_rect(
            shadow,
            10,
            Color::rgba(theme.bg.r(), theme.bg.g(), theme.bg.b(), 140),
        );
        canvas.fill_rounded_rect(page, 10, Color::rgb(0xFB, 0xFA, 0xF7));
        canvas.stroke_rounded_rect(page, 10, 1, Color::rgb(0xD7, 0xD3, 0xCD));

        let page_top = Rect::new(page.x, page.y, page.w, 62);
        fill_vertical_gradient(
            canvas,
            page_top,
            Color::rgb(0xFF, 0xFF, 0xFF),
            Color::rgb(0xF4, 0xF1, 0xED),
        );
        canvas.hbar(page.x, page.y + 61, page.w, 1, Color::rgb(0xE7, 0xE1, 0xD8));

        draw_text(
            canvas,
            "Sunlight Canvas Area",
            page.x + 28,
            page.y + 22,
            &TextStyle::new(FontRole::UiLarge, Color::rgb(0x24, 0x24, 0x28)),
        );
        draw_text(
            canvas,
            "Reserved for the future canvas widget and document surface",
            page.x + 28,
            page.y + 40,
            &TextStyle::new(FontRole::UiSmall, Color::rgb(0x72, 0x72, 0x7C)),
        );

        let insert_rect = self.canvas_insertion_rect();
        canvas.fill_rounded_rect(insert_rect, 12, Color::rgb(0xFF, 0xFF, 0xFF));
        canvas.stroke_rounded_rect(insert_rect, 12, 1, Color::rgb(0xE0, 0xDB, 0xD4));
        canvas.hbar(
            insert_rect.x + 1,
            insert_rect.y + 1,
            insert_rect.w - 2,
            4,
            theme.accent.lighten(34),
        );

        for idx in 0..18 {
            let y = insert_rect.y + 56 + idx * 26;
            if y + 1 >= insert_rect.bottom() - 32 {
                break;
            }
            let line_w = insert_rect.w as i32 - 92 - ((idx % 3) * 38);
            canvas.fill_rect(
                Rect::new(insert_rect.x + 30, y, line_w.max(120) as u32, 2),
                Color::rgb(0xEB, 0xE6, 0xDF),
            );
        }

        let badge_w = measure_text("Canvas Widget Placeholder", FontRole::UiMedium).w + 28;
        let badge = Rect::new(
            insert_rect.x + ((insert_rect.w as i32 - badge_w as i32) / 2),
            insert_rect.y + insert_rect.h as i32 / 2 - 18,
            badge_w,
            36,
        );
        canvas.fill_rounded_rect(badge, 18, theme.panel);
        canvas.stroke_rounded_rect(badge, 18, 1, theme.accent.darken(80));
        draw_text_vcenter(
            canvas,
            "Canvas Widget Placeholder",
            badge.x + 14,
            badge.y,
            badge.h,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );

        let footer = Rect::new(page.x + 28, page.bottom() - 44, page.w - 56, 20);
        draw_text(
            canvas,
            "Future integration point: replace placeholder drawing inside `canvas_insertion_rect()`.",
            footer.x,
            footer.y,
            &TextStyle::new(FontRole::UiSmall, Color::rgb(0x86, 0x82, 0x7B)),
        );
    }

    fn draw_status_bar(&self, canvas: &mut Canvas, theme: &Theme) {
        StatusBar::new(
            self.status_rect(),
            "Page 1 of 1 | Col 1",
            self.status_center.as_str(),
            "100% | Canvas Widget Inactive | WYSIWYG",
        )
        .draw(canvas, theme);
    }
}

impl App for WriterApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);
        self.draw_top_bar(canvas, theme);
        self.draw_ribbon(canvas, theme);
        self.draw_canvas_placeholder(canvas, theme);
        self.draw_status_bar(canvas, theme);
        self.draw_app_menu(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Tick => {
                if self.status_ticks > 0 {
                    self.status_ticks -= 1;
                    if self.status_ticks == 0 {
                        self.status_center.clear();
                        self.status_center.set("Canvas Widget Placeholder");
                        return true;
                    }
                }
                false
            }
            Event::MouseMove { x, y } => {
                let point = Point::new(x, y);
                let mut redraw = false;

                let menu_hover = self.menu_button_rect().contains(point);
                if menu_hover != self.menu_button_hover {
                    self.menu_button_hover = menu_hover;
                    redraw = true;
                }

                let quick_hover = self.quick_chip_hit(point);
                if quick_hover != self.quick_hover {
                    self.quick_hover = quick_hover;
                    redraw = true;
                }

                let ribbon_hover = self.ribbon_hit(point);
                if ribbon_hover != self.ribbon_hover {
                    self.ribbon_hover = ribbon_hover;
                    redraw = true;
                }

                if self.menu_open {
                    let new_menu_hover = self.menu_item_hit(point);
                    let new_recent_hover = self.recent_doc_hit(point);
                    if new_menu_hover != self.menu_hover || new_recent_hover != self.recent_hover {
                        self.menu_hover = new_menu_hover;
                        self.recent_hover = new_recent_hover;
                        redraw = true;
                    }
                }

                redraw
            }
            Event::Click { x, y } => {
                let point = Point::new(x, y);

                if self.menu_button_rect().contains(point) {
                    self.toggle_menu();
                    return true;
                }

                if self.menu_open {
                    if let Some(idx) = self.recent_doc_hit(point) {
                        self.close_menu();
                        return self.dispatch_action(WriterAction::RecentDocument(idx));
                    }

                    if let Some(idx) = self.menu_item_hit(point) {
                        let item = APP_MENU_ITEMS[idx];
                        if item.submenu {
                            self.menu_pinned = Some(idx);
                            self.menu_hover = Some(idx);
                            self.set_status_message("Open recent documents");
                            return true;
                        }
                        self.close_menu();
                        return self.dispatch_action(item.action);
                    }

                    if !self.app_menu_rect().contains(point) {
                        self.close_menu();
                        return true;
                    }
                }

                if let Some((group_idx, control_idx)) = self.ribbon_hit(point) {
                    return self.dispatch_action(RIBBON_GROUPS[group_idx].controls[control_idx].action);
                }

                false
            }
            Event::KeyPress {
                keycode,
                pressed,
                ..
            } => {
                if !pressed {
                    return false;
                }
                if keycode == KEY_ESC {
                    if self.menu_open {
                        self.close_menu();
                        self.set_status_message("Application menu closed");
                        return true;
                    }
                    request_close();
                }
                false
            }
            _ => false,
        }
    }
}

fn fill_vertical_gradient(canvas: &mut Canvas, rect: Rect, top: Color, bottom: Color) {
    let h = rect.h.max(1);
    for row in 0..h {
        let mix = row * 255 / h;
        let r = ((top.r() as u32 * (255 - mix) + bottom.r() as u32 * mix) / 255) as u8;
        let g = ((top.g() as u32 * (255 - mix) + bottom.g() as u32 * mix) / 255) as u8;
        let b = ((top.b() as u32 * (255 - mix) + bottom.b() as u32 * mix) / 255) as u8;
        canvas.fill_rect(
            Rect::new(rect.x, rect.y + row as i32, rect.w, 1),
            Color::rgb(r, g, b),
        );
    }
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    sunlight_libc::launch_trace::init_from_argv(argc, argv);
    let trace = launch_trace::current().unwrap_or(LaunchTrace::new(0, LaunchSource::Unknown, 0));
    launch_trace::log_phase_now(
        trace,
        "app=sunlight-writer",
        "app_main_started",
        Some(sunlight_ipc::getpid()),
    );

    let mut app = WriterApp::new();
    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Sunlight Writer",
        decoration: WindowDecoration::Normal,
    }) {
        Some(window) => window,
        None => {
            debug_log("[WRITER] failed to connect window\n");
            loop {
                process_yield();
            }
        }
    };

    window.run(&mut app);
    ProcessExit::exit(0);
}
