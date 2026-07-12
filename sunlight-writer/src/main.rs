#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![cfg_attr(test, allow(dead_code, unused_imports))]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
#[cfg(not(test))]
use core::alloc::GlobalAlloc;

use sun_font::{FontRole, VecFont};
use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    process_yield, ProcessExit,
};
use sunlight_ui::image::TgaImage;
use sunlight_ui::widgets::{
    AppMenuCommand, AppMenuSecondaryItem, CanvasHitTarget, DocumentCanvas, DocumentCanvasItem,
    DocumentCanvasMode, DocumentCanvasPresentation, DocumentRectStyle, DocumentStrokeStyle,
    DocumentTextStyle, HeaderActionButton, HeaderChip, PremiumHeader, RibbonBar, RibbonButtonKind,
    RibbonButtonSpec, RibbonGroupSpec, StatusBar, TextEditState, TextLineLayout,
    TwoPaneAppMenu, find_line_index, layout_text_lines, caret_x_on_line, byte_at_x_on_line,
    line_home_byte, line_end_byte, click_to_line_and_byte,
};
use sunlight_ui::{
    request_close, set_client_cursor, App, Color, CursorShape, Event, Point, Rect, Theme, Window,
    WindowConfig, WindowDecoration,
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
const MSG_LEN: usize = 96;
const KEY_ESC: u8 = 0x01;

const KEY_LEFT: u8 = 0x4B;
const KEY_RIGHT: u8 = 0x4D;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_HOME: u8 = 0x47;
const KEY_END: u8 = 0x4F;
const KEY_DELETE: u8 = 0x53;

const SAMPLE_EDITABLE_TEXT: &str = "SunlightOS ☀️  Rabbit 🐇  Penguin 🐧  Rust 🦀";

const EDITABLE_ITEM_INDEX: usize = 5;

static FONT_UI_TITLE: VecFont = VecFont(FontRole::UiTitle);
static FONT_UI_LARGE: VecFont = VecFont(FontRole::UiLarge);
static FONT_UI_MEDIUM: VecFont = VecFont(FontRole::UiMedium);
static FONT_UI_REGULAR: VecFont = VecFont(FontRole::UiRegular);
static FONT_UI_SMALL: VecFont = VecFont(FontRole::UiSmall);
static FONT_SERIF: VecFont = VecFont(FontRole::SerifRegular);

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
static ICON_NUMBERING_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_numbering.tga"));
static ICON_PICTURE_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_picture.tga"));
static ICON_LINK_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_link.tga"));

#[cfg(not(test))]
struct BumpAllocator;
#[cfg(not(test))]
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

#[cfg(not(test))]
#[global_allocator]
static ALLOC: BumpAllocator = BumpAllocator;

#[cfg(not(test))]
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
struct AppMenuItemDef {
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
struct QuickChipDef {
    label: &'static str,
    icon: Option<IconId>,
    width: u32,
    accent_outline: bool,
}

#[derive(Clone, Copy)]
struct RibbonCommandDef {
    label: &'static str,
    icon: Option<IconId>,
    width: u32,
    kind: RibbonButtonKind,
    row: u8,
    action: WriterAction,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum WriterTextRole {
    Title,
    Subtitle,
    Paragraph,
    Callout,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WriterRectRole {
    Callout,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WriterLineRole {
    Divider,
}

#[derive(Clone, Copy)]
enum WriterBlock<'a> {
    Text {
        x: i32,
        y: i32,
        text: &'a str,
        role: WriterTextRole,
    },
    Link {
        x: i32,
        y: i32,
        text: &'a str,
        url: &'a str,
    },
    Rect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        role: WriterRectRole,
    },
    Line {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        role: WriterLineRole,
    },
    ImagePlaceholder {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        label: &'a str,
    },
}

#[derive(Clone, Copy)]
struct WriterDocument<'a> {
    mode: DocumentCanvasMode,
    blocks: &'a [WriterBlock<'a>],
}

const SAMPLE_DOCUMENT_BLOCKS: &[WriterBlock<'static>] = &[
    WriterBlock::Text {
        x: 0,
        y: 0,
        text: "Sunlight Writer document surface",
        role: WriterTextRole::Title,
    },
    WriterBlock::Text {
        x: 0,
        y: 32,
        text: "Reusable fixed-coordinate canvas for Writer, Notes, previews, and future read-only viewers.",
        role: WriterTextRole::Subtitle,
    },
    WriterBlock::Line {
        x1: 0,
        y1: 58,
        x2: 676,
        y2: 58,
        role: WriterLineRole::Divider,
    },
    WriterBlock::Text {
        x: 0,
        y: 96,
        text: "This first patch keeps the polished Writer shell intact and swaps only the central placeholder for a shared page widget.",
        role: WriterTextRole::Paragraph,
    },
    WriterBlock::Text {
        x: 0,
        y: 136,
        text: "The widget renders a real document page, comfortable margins, subtle guide lines, and a stable primitive list instead of layout logic.",
        role: WriterTextRole::Paragraph,
    },
    WriterBlock::Text {
        x: 0,
        y: 166,
        text: "SunlightOS ☀️  Rabbit 🐇  Penguin 🐧  Rust 🦀",
        role: WriterTextRole::Paragraph,
    },
    WriterBlock::Rect {
        x: 0,
        y: 186,
        w: 310,
        h: 54,
        role: WriterRectRole::Callout,
    },
    WriterBlock::Text {
        x: 18,
        y: 204,
        text: "Mode: Editable  |  Rendering: fixed coordinates",
        role: WriterTextRole::Callout,
    },
    WriterBlock::Link {
        x: 0,
        y: 268,
        text: "Future feed: absolute-position document items from Golden Fish and office-style apps.",
        url: "sunlight://document-canvas",
    },
    WriterBlock::ImagePlaceholder {
        x: 438,
        y: 186,
        w: 238,
        h: 164,
        label: "Image / preview placeholder",
    },
];

impl<'a> WriterDocument<'a> {
    fn sample() -> WriterDocument<'static> {
        WriterDocument {
            mode: DocumentCanvasMode::Editable,
            blocks: SAMPLE_DOCUMENT_BLOCKS,
        }
    }

    #[cfg(test)]
    fn empty(mode: DocumentCanvasMode) -> Self {
        Self { mode, blocks: &[] }
    }

    #[allow(dead_code)]
    fn to_canvas_items(&self) -> Vec<DocumentCanvasItem<'a>> {
        let mut items = Vec::with_capacity(self.blocks.len());
        for block in self.blocks {
            match *block {
                WriterBlock::Text { x, y, text, role } => {
                    items.push(DocumentCanvasItem::Text {
                        x,
                        y,
                        text,
                        style: writer_text_style(role),
                    });
                }
                WriterBlock::Link { x, y, text, url } => {
                    items.push(DocumentCanvasItem::LinkText {
                        x,
                        y,
                        text,
                        url,
                        style: writer_link_style(),
                    });
                }
                WriterBlock::Rect { x, y, w, h, role } => {
                    items.push(DocumentCanvasItem::Rect {
                        x,
                        y,
                        w,
                        h,
                        style: writer_rect_style(role),
                    });
                }
                WriterBlock::Line {
                    x1,
                    y1,
                    x2,
                    y2,
                    role,
                } => {
                    items.push(DocumentCanvasItem::Line {
                        x1,
                        y1,
                        x2,
                        y2,
                        style: writer_line_style(role),
                    });
                }
                WriterBlock::ImagePlaceholder { x, y, w, h, label } => {
                    items.push(DocumentCanvasItem::ImagePlaceholder { x, y, w, h, label });
                }
            }
        }
        items
    }
}

fn writer_text_style(role: WriterTextRole) -> DocumentTextStyle<'static> {
    match role {
        WriterTextRole::Title => {
            DocumentTextStyle::new(Some(&FONT_UI_LARGE), Color::rgb(0x25, 0x25, 0x29))
        }
        WriterTextRole::Subtitle => {
            DocumentTextStyle::new(Some(&FONT_UI_SMALL), Color::rgb(0x6C, 0x6B, 0x73))
        }
        WriterTextRole::Paragraph => {
            DocumentTextStyle::new(Some(&FONT_UI_MEDIUM), Color::rgb(0x37, 0x37, 0x3C))
        }
        WriterTextRole::Callout => {
            // Keep the existing editable document model intact while exposing
            // the native serif face in the current callout style.
            DocumentTextStyle::new(Some(&FONT_SERIF), Color::rgb(0x7A, 0x64, 0x34))
        }
    }
}

fn writer_link_style() -> DocumentTextStyle<'static> {
    DocumentTextStyle::new(Some(&FONT_UI_MEDIUM), Color::rgb(0xA6, 0x5E, 0x00))
}

fn writer_rect_style(role: WriterRectRole) -> DocumentRectStyle {
    match role {
        WriterRectRole::Callout => DocumentRectStyle::new(
            Color::rgb(0xFA, 0xF6, 0xEF),
            Some(DocumentStrokeStyle::new(Color::rgb(0xE5, 0xDB, 0xC8), 1)),
        ),
    }
}

fn writer_line_style(role: WriterLineRole) -> DocumentStrokeStyle {
    match role {
        WriterLineRole::Divider => DocumentStrokeStyle::new(Color::rgb(0xDD, 0xD7, 0xCF), 1),
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

const APP_MENU_ITEMS: [AppMenuItemDef; 8] = [
    AppMenuItemDef {
        label: "New",
        action: WriterAction::New,
        icon: Some(IconId::New),
        submenu: false,
    },
    AppMenuItemDef {
        label: "Open",
        action: WriterAction::Open,
        icon: Some(IconId::Open),
        submenu: true,
    },
    AppMenuItemDef {
        label: "Save",
        action: WriterAction::Save,
        icon: Some(IconId::Save),
        submenu: false,
    },
    AppMenuItemDef {
        label: "Save As",
        action: WriterAction::SaveAs,
        icon: Some(IconId::Save),
        submenu: false,
    },
    AppMenuItemDef {
        label: "Print",
        action: WriterAction::Print,
        icon: Some(IconId::Print),
        submenu: false,
    },
    AppMenuItemDef {
        label: "Share",
        action: WriterAction::Share,
        icon: Some(IconId::Share),
        submenu: false,
    },
    AppMenuItemDef {
        label: "Export",
        action: WriterAction::Export,
        icon: Some(IconId::Share),
        submenu: false,
    },
    AppMenuItemDef {
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

const QUICK_CHIPS: [QuickChipDef; 3] = [
    QuickChipDef {
        label: "Secure Draft",
        icon: Some(IconId::Doc),
        width: 104,
        accent_outline: false,
    },
    QuickChipDef {
        label: "Premium Workspace",
        icon: None,
        width: 144,
        accent_outline: true,
    },
    QuickChipDef {
        label: "Canvas Pending",
        icon: None,
        width: 132,
        accent_outline: false,
    },
];

const FILE_GROUP_DEFS: [RibbonCommandDef; 4] = [
    RibbonCommandDef {
        label: "New",
        icon: Some(IconId::New),
        width: 78,
        kind: RibbonButtonKind::WideButton,
        row: 0,
        action: WriterAction::New,
    },
    RibbonCommandDef {
        label: "Open",
        icon: Some(IconId::Open),
        width: 78,
        kind: RibbonButtonKind::WideButton,
        row: 0,
        action: WriterAction::Open,
    },
    RibbonCommandDef {
        label: "Save",
        icon: Some(IconId::Save),
        width: 78,
        kind: RibbonButtonKind::WideButton,
        row: 1,
        action: WriterAction::Save,
    },
    RibbonCommandDef {
        label: "Print",
        icon: Some(IconId::Print),
        width: 78,
        kind: RibbonButtonKind::WideButton,
        row: 1,
        action: WriterAction::Print,
    },
];

const FONT_GROUP_DEFS: [RibbonCommandDef; 5] = [
    RibbonCommandDef {
        label: "Inter",
        icon: None,
        width: 124,
        kind: RibbonButtonKind::Dropdown,
        row: 0,
        action: WriterAction::FontFamily,
    },
    RibbonCommandDef {
        label: "12",
        icon: None,
        width: 56,
        kind: RibbonButtonKind::Dropdown,
        row: 0,
        action: WriterAction::FontSize,
    },
    RibbonCommandDef {
        label: "",
        icon: Some(IconId::Bold),
        width: 40,
        kind: RibbonButtonKind::Toggle,
        row: 1,
        action: WriterAction::Bold,
    },
    RibbonCommandDef {
        label: "",
        icon: Some(IconId::Italic),
        width: 40,
        kind: RibbonButtonKind::Toggle,
        row: 1,
        action: WriterAction::Italic,
    },
    RibbonCommandDef {
        label: "",
        icon: Some(IconId::Underline),
        width: 40,
        kind: RibbonButtonKind::Toggle,
        row: 1,
        action: WriterAction::Underline,
    },
];

const PARAGRAPH_GROUP_DEFS: [RibbonCommandDef; 6] = [
    RibbonCommandDef {
        label: "",
        icon: Some(IconId::AlignLeft),
        width: 40,
        kind: RibbonButtonKind::IconButton,
        row: 0,
        action: WriterAction::AlignLeft,
    },
    RibbonCommandDef {
        label: "",
        icon: Some(IconId::AlignCenter),
        width: 40,
        kind: RibbonButtonKind::IconButton,
        row: 0,
        action: WriterAction::AlignCenter,
    },
    RibbonCommandDef {
        label: "",
        icon: Some(IconId::AlignRight),
        width: 40,
        kind: RibbonButtonKind::IconButton,
        row: 0,
        action: WriterAction::AlignRight,
    },
    RibbonCommandDef {
        label: "",
        icon: Some(IconId::AlignJustify),
        width: 40,
        kind: RibbonButtonKind::IconButton,
        row: 1,
        action: WriterAction::AlignJustify,
    },
    RibbonCommandDef {
        label: "",
        icon: Some(IconId::Bullets),
        width: 40,
        kind: RibbonButtonKind::IconButton,
        row: 1,
        action: WriterAction::Bullets,
    },
    RibbonCommandDef {
        label: "",
        icon: Some(IconId::Numbering),
        width: 40,
        kind: RibbonButtonKind::IconButton,
        row: 1,
        action: WriterAction::Numbering,
    },
];

const INSERT_GROUP_DEFS: [RibbonCommandDef; 4] = [
    RibbonCommandDef {
        label: "Picture",
        icon: Some(IconId::Picture),
        width: 92,
        kind: RibbonButtonKind::WideButton,
        row: 0,
        action: WriterAction::InsertPicture,
    },
    RibbonCommandDef {
        label: "Table",
        icon: None,
        width: 76,
        kind: RibbonButtonKind::WideButton,
        row: 0,
        action: WriterAction::InsertTable,
    },
    RibbonCommandDef {
        label: "Shape",
        icon: None,
        width: 76,
        kind: RibbonButtonKind::WideButton,
        row: 1,
        action: WriterAction::InsertShape,
    },
    RibbonCommandDef {
        label: "Link",
        icon: Some(IconId::Link),
        width: 76,
        kind: RibbonButtonKind::WideButton,
        row: 1,
        action: WriterAction::InsertLink,
    },
];

struct WriterApp {
    icons: WriterIcons,
    document: WriterDocument<'static>,
    menu_open: bool,
    menu_hover: Option<usize>,
    menu_pinned: Option<usize>,
    recent_hover: Option<usize>,
    quick_hover: Option<usize>,
    ribbon_hover: Option<(usize, usize)>,
    menu_button_hover: bool,
    status_center: TextSlot,
    status_ticks: u16,
    edit_buffer: String,
    edit_state: TextEditState,
    document_modified: bool,
    prev_document_cursor: CursorShape,
}

impl WriterApp {
    fn new() -> Self {
        let mut status_center = TextSlot::empty();
        status_center.set("Document Canvas Ready");
        Self {
            icons: WriterIcons::load(),
            document: WriterDocument::sample(),
            menu_open: false,
            menu_hover: None,
            menu_pinned: None,
            recent_hover: None,
            quick_hover: None,
            ribbon_hover: None,
            menu_button_hover: false,
            status_center,
            status_ticks: 0,
            edit_buffer: String::from(SAMPLE_EDITABLE_TEXT),
            edit_state: TextEditState::default(),
            document_modified: false,
            prev_document_cursor: CursorShape::Pointer,
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
        Rect::new(0, top as i32, WIN_W, WIN_H.saturating_sub(top + STATUS_H))
    }

    fn status_rect(&self) -> Rect {
        Rect::new(0, (WIN_H - STATUS_H) as i32, WIN_W, STATUS_H)
    }

    fn menu_visible_secondary(&self) -> bool {
        self.menu_hover.or(self.menu_pinned) == Some(1)
    }

    fn app_menu_rect(&self) -> Rect {
        let width = if self.menu_visible_secondary() {
            APP_MENU_LEFT_W + APP_MENU_RIGHT_W
        } else {
            APP_MENU_LEFT_W
        };
        Rect::new(
            APP_MENU_X,
            self.top_bar_rect().bottom() + APP_MENU_Y_GAP,
            width,
            34 + APP_MENU_ITEMS.len() as u32 * 30 + 12,
        )
    }

    fn icon(&self, id: IconId) -> Option<&TgaImage> {
        self.icons.get(id)
    }

    fn with_header<T>(&self, f: impl FnOnce(PremiumHeader<'_>) -> T) -> T {
        let chips = [
            HeaderChip {
                label: QUICK_CHIPS[0].label,
                icon: QUICK_CHIPS[0].icon.and_then(|id| self.icon(id)),
                width: QUICK_CHIPS[0].width,
                accent_outline: QUICK_CHIPS[0].accent_outline,
            },
            HeaderChip {
                label: QUICK_CHIPS[1].label,
                icon: QUICK_CHIPS[1].icon.and_then(|id| self.icon(id)),
                width: QUICK_CHIPS[1].width,
                accent_outline: QUICK_CHIPS[1].accent_outline,
            },
            HeaderChip {
                label: QUICK_CHIPS[2].label,
                icon: QUICK_CHIPS[2].icon.and_then(|id| self.icon(id)),
                width: QUICK_CHIPS[2].width,
                accent_outline: QUICK_CHIPS[2].accent_outline,
            },
        ];
        let button = self.icon(IconId::Menu).map(|icon| HeaderActionButton {
            rect: Rect::new(APP_MENU_X, 10, 44, 32),
            icon,
            active: self.menu_open,
            hovered: self.menu_button_hover,
        });
        f(PremiumHeader {
            rect: self.top_bar_rect(),
            title: "Sunlight Writer",
            subtitle: "Professional document shell . ribbon workspace . canvas-ready layout",
            leading_button: button,
            chips: &chips,
            hovered_chip: self.quick_hover,
            title_font: Some(&FONT_UI_TITLE),
            subtitle_font: Some(&FONT_UI_SMALL),
            chip_font: Some(&FONT_UI_SMALL),
        })
    }

    fn with_app_menu<T>(&self, f: impl FnOnce(TwoPaneAppMenu<'_>) -> T) -> T {
        let commands = [
            AppMenuCommand {
                label: APP_MENU_ITEMS[0].label,
                icon: APP_MENU_ITEMS[0].icon.and_then(|id| self.icon(id)),
                has_secondary: APP_MENU_ITEMS[0].submenu,
            },
            AppMenuCommand {
                label: APP_MENU_ITEMS[1].label,
                icon: APP_MENU_ITEMS[1].icon.and_then(|id| self.icon(id)),
                has_secondary: APP_MENU_ITEMS[1].submenu,
            },
            AppMenuCommand {
                label: APP_MENU_ITEMS[2].label,
                icon: APP_MENU_ITEMS[2].icon.and_then(|id| self.icon(id)),
                has_secondary: APP_MENU_ITEMS[2].submenu,
            },
            AppMenuCommand {
                label: APP_MENU_ITEMS[3].label,
                icon: APP_MENU_ITEMS[3].icon.and_then(|id| self.icon(id)),
                has_secondary: APP_MENU_ITEMS[3].submenu,
            },
            AppMenuCommand {
                label: APP_MENU_ITEMS[4].label,
                icon: APP_MENU_ITEMS[4].icon.and_then(|id| self.icon(id)),
                has_secondary: APP_MENU_ITEMS[4].submenu,
            },
            AppMenuCommand {
                label: APP_MENU_ITEMS[5].label,
                icon: APP_MENU_ITEMS[5].icon.and_then(|id| self.icon(id)),
                has_secondary: APP_MENU_ITEMS[5].submenu,
            },
            AppMenuCommand {
                label: APP_MENU_ITEMS[6].label,
                icon: APP_MENU_ITEMS[6].icon.and_then(|id| self.icon(id)),
                has_secondary: APP_MENU_ITEMS[6].submenu,
            },
            AppMenuCommand {
                label: APP_MENU_ITEMS[7].label,
                icon: APP_MENU_ITEMS[7].icon.and_then(|id| self.icon(id)),
                has_secondary: APP_MENU_ITEMS[7].submenu,
            },
        ];
        let recent = [
            AppMenuSecondaryItem {
                title: RECENT_DOCS[0].title,
                subtitle: RECENT_DOCS[0].meta,
                icon: self.icon(IconId::Doc),
            },
            AppMenuSecondaryItem {
                title: RECENT_DOCS[1].title,
                subtitle: RECENT_DOCS[1].meta,
                icon: self.icon(IconId::Doc),
            },
            AppMenuSecondaryItem {
                title: RECENT_DOCS[2].title,
                subtitle: RECENT_DOCS[2].meta,
                icon: self.icon(IconId::Doc),
            },
            AppMenuSecondaryItem {
                title: RECENT_DOCS[3].title,
                subtitle: RECENT_DOCS[3].meta,
                icon: self.icon(IconId::Doc),
            },
        ];
        f(TwoPaneAppMenu {
            rect: self.app_menu_rect(),
            left_width: APP_MENU_LEFT_W,
            right_width: APP_MENU_RIGHT_W,
            header_title: "Application Menu",
            header_subtitle: "Writer shell commands",
            secondary_title: "Recent Documents",
            secondary_subtitle: "Open continues into this column",
            commands: &commands,
            secondary_items: &recent,
            active_command: self.menu_hover.or(self.menu_pinned),
            active_secondary: self.recent_hover,
            show_secondary: self.menu_visible_secondary(),
            title_font: Some(&FONT_UI_MEDIUM),
            label_font: Some(&FONT_UI_REGULAR),
            small_font: Some(&FONT_UI_SMALL),
        })
    }

    fn with_ribbon_bar<T>(&self, f: impl FnOnce(RibbonBar<'_>) -> T) -> T {
        let file = [
            RibbonButtonSpec {
                label: FILE_GROUP_DEFS[0].label,
                icon: FILE_GROUP_DEFS[0].icon.and_then(|id| self.icon(id)),
                width: FILE_GROUP_DEFS[0].width,
                kind: FILE_GROUP_DEFS[0].kind,
                row: FILE_GROUP_DEFS[0].row,
            },
            RibbonButtonSpec {
                label: FILE_GROUP_DEFS[1].label,
                icon: FILE_GROUP_DEFS[1].icon.and_then(|id| self.icon(id)),
                width: FILE_GROUP_DEFS[1].width,
                kind: FILE_GROUP_DEFS[1].kind,
                row: FILE_GROUP_DEFS[1].row,
            },
            RibbonButtonSpec {
                label: FILE_GROUP_DEFS[2].label,
                icon: FILE_GROUP_DEFS[2].icon.and_then(|id| self.icon(id)),
                width: FILE_GROUP_DEFS[2].width,
                kind: FILE_GROUP_DEFS[2].kind,
                row: FILE_GROUP_DEFS[2].row,
            },
            RibbonButtonSpec {
                label: FILE_GROUP_DEFS[3].label,
                icon: FILE_GROUP_DEFS[3].icon.and_then(|id| self.icon(id)),
                width: FILE_GROUP_DEFS[3].width,
                kind: FILE_GROUP_DEFS[3].kind,
                row: FILE_GROUP_DEFS[3].row,
            },
        ];
        let font = [
            RibbonButtonSpec {
                label: FONT_GROUP_DEFS[0].label,
                icon: None,
                width: FONT_GROUP_DEFS[0].width,
                kind: FONT_GROUP_DEFS[0].kind,
                row: FONT_GROUP_DEFS[0].row,
            },
            RibbonButtonSpec {
                label: FONT_GROUP_DEFS[1].label,
                icon: None,
                width: FONT_GROUP_DEFS[1].width,
                kind: FONT_GROUP_DEFS[1].kind,
                row: FONT_GROUP_DEFS[1].row,
            },
            RibbonButtonSpec {
                label: "",
                icon: FONT_GROUP_DEFS[2].icon.and_then(|id| self.icon(id)),
                width: FONT_GROUP_DEFS[2].width,
                kind: FONT_GROUP_DEFS[2].kind,
                row: FONT_GROUP_DEFS[2].row,
            },
            RibbonButtonSpec {
                label: "",
                icon: FONT_GROUP_DEFS[3].icon.and_then(|id| self.icon(id)),
                width: FONT_GROUP_DEFS[3].width,
                kind: FONT_GROUP_DEFS[3].kind,
                row: FONT_GROUP_DEFS[3].row,
            },
            RibbonButtonSpec {
                label: "",
                icon: FONT_GROUP_DEFS[4].icon.and_then(|id| self.icon(id)),
                width: FONT_GROUP_DEFS[4].width,
                kind: FONT_GROUP_DEFS[4].kind,
                row: FONT_GROUP_DEFS[4].row,
            },
        ];
        let paragraph = [
            RibbonButtonSpec {
                label: "",
                icon: PARAGRAPH_GROUP_DEFS[0].icon.and_then(|id| self.icon(id)),
                width: PARAGRAPH_GROUP_DEFS[0].width,
                kind: PARAGRAPH_GROUP_DEFS[0].kind,
                row: PARAGRAPH_GROUP_DEFS[0].row,
            },
            RibbonButtonSpec {
                label: "",
                icon: PARAGRAPH_GROUP_DEFS[1].icon.and_then(|id| self.icon(id)),
                width: PARAGRAPH_GROUP_DEFS[1].width,
                kind: PARAGRAPH_GROUP_DEFS[1].kind,
                row: PARAGRAPH_GROUP_DEFS[1].row,
            },
            RibbonButtonSpec {
                label: "",
                icon: PARAGRAPH_GROUP_DEFS[2].icon.and_then(|id| self.icon(id)),
                width: PARAGRAPH_GROUP_DEFS[2].width,
                kind: PARAGRAPH_GROUP_DEFS[2].kind,
                row: PARAGRAPH_GROUP_DEFS[2].row,
            },
            RibbonButtonSpec {
                label: "",
                icon: PARAGRAPH_GROUP_DEFS[3].icon.and_then(|id| self.icon(id)),
                width: PARAGRAPH_GROUP_DEFS[3].width,
                kind: PARAGRAPH_GROUP_DEFS[3].kind,
                row: PARAGRAPH_GROUP_DEFS[3].row,
            },
            RibbonButtonSpec {
                label: "",
                icon: PARAGRAPH_GROUP_DEFS[4].icon.and_then(|id| self.icon(id)),
                width: PARAGRAPH_GROUP_DEFS[4].width,
                kind: PARAGRAPH_GROUP_DEFS[4].kind,
                row: PARAGRAPH_GROUP_DEFS[4].row,
            },
            RibbonButtonSpec {
                label: "",
                icon: PARAGRAPH_GROUP_DEFS[5].icon.and_then(|id| self.icon(id)),
                width: PARAGRAPH_GROUP_DEFS[5].width,
                kind: PARAGRAPH_GROUP_DEFS[5].kind,
                row: PARAGRAPH_GROUP_DEFS[5].row,
            },
        ];
        let insert = [
            RibbonButtonSpec {
                label: INSERT_GROUP_DEFS[0].label,
                icon: INSERT_GROUP_DEFS[0].icon.and_then(|id| self.icon(id)),
                width: INSERT_GROUP_DEFS[0].width,
                kind: INSERT_GROUP_DEFS[0].kind,
                row: INSERT_GROUP_DEFS[0].row,
            },
            RibbonButtonSpec {
                label: INSERT_GROUP_DEFS[1].label,
                icon: None,
                width: INSERT_GROUP_DEFS[1].width,
                kind: INSERT_GROUP_DEFS[1].kind,
                row: INSERT_GROUP_DEFS[1].row,
            },
            RibbonButtonSpec {
                label: INSERT_GROUP_DEFS[2].label,
                icon: None,
                width: INSERT_GROUP_DEFS[2].width,
                kind: INSERT_GROUP_DEFS[2].kind,
                row: INSERT_GROUP_DEFS[2].row,
            },
            RibbonButtonSpec {
                label: INSERT_GROUP_DEFS[3].label,
                icon: INSERT_GROUP_DEFS[3].icon.and_then(|id| self.icon(id)),
                width: INSERT_GROUP_DEFS[3].width,
                kind: INSERT_GROUP_DEFS[3].kind,
                row: INSERT_GROUP_DEFS[3].row,
            },
        ];
        let groups = [
            RibbonGroupSpec {
                title: "File",
                buttons: &file,
            },
            RibbonGroupSpec {
                title: "Font",
                buttons: &font,
            },
            RibbonGroupSpec {
                title: "Paragraph",
                buttons: &paragraph,
            },
            RibbonGroupSpec {
                title: "Insert",
                buttons: &insert,
            },
        ];
        f(RibbonBar {
            rect: self.ribbon_rect(),
            groups: &groups,
            hovered: self.ribbon_hover,
            label_font: Some(&FONT_UI_REGULAR),
            small_font: Some(&FONT_UI_SMALL),
        })
    }

    fn document_items(&self) -> Vec<DocumentCanvasItem<'_>> {
        let mut items = Vec::with_capacity(self.document.blocks.len());
        for (idx, block) in self.document.blocks.iter().enumerate() {
            match *block {
                WriterBlock::Text { x, y, text, role } => {
                    let resolved_text: &str = if idx == EDITABLE_ITEM_INDEX {
                        self.edit_buffer.as_str()
                    } else {
                        text
                    };
                    items.push(DocumentCanvasItem::Text {
                        x,
                        y,
                        text: resolved_text,
                        style: writer_text_style(role),
                    });
                }
                WriterBlock::Link { x, y, text, url } => {
                    items.push(DocumentCanvasItem::LinkText {
                        x,
                        y,
                        text,
                        url,
                        style: writer_link_style(),
                    });
                }
                WriterBlock::Rect { x, y, w, h, role } => {
                    items.push(DocumentCanvasItem::Rect {
                        x,
                        y,
                        w,
                        h,
                        style: writer_rect_style(role),
                    });
                }
                WriterBlock::Line {
                    x1,
                    y1,
                    x2,
                    y2,
                    role,
                } => {
                    items.push(DocumentCanvasItem::Line {
                        x1,
                        y1,
                        x2,
                        y2,
                        style: writer_line_style(role),
                    });
                }
                WriterBlock::ImagePlaceholder { x, y, w, h, label } => {
                    items.push(DocumentCanvasItem::ImagePlaceholder { x, y, w, h, label });
                }
            }
        }
        items
    }

    fn document_canvas<'a>(&'a self, items: &'a [DocumentCanvasItem<'a>]) -> DocumentCanvas<'a> {
        DocumentCanvas::new(self.content_rect(), items)
            .with_mode(self.document.mode)
            .with_presentation(DocumentCanvasPresentation::Writer)
            .with_empty_label("Document Canvas Ready")
            .with_fonts(
                Some(&FONT_UI_LARGE),
                Some(&FONT_UI_SMALL),
                Some(&FONT_UI_MEDIUM),
                Some(&FONT_UI_SMALL),
            )
            .with_edit_state(&self.edit_state)
    }

    fn quick_chip_hit(&self, point: Point) -> Option<usize> {
        self.with_header(|header| header.chip_hit(point))
    }

    fn menu_button_hit(&self, point: Point) -> bool {
        self.with_header(|header| header.leading_button_hit(point))
    }

    fn menu_command_hit(&self, point: Point) -> Option<usize> {
        self.with_app_menu(|menu| menu.command_hit(point))
    }

    fn recent_doc_hit(&self, point: Point) -> Option<usize> {
        self.with_app_menu(|menu| menu.secondary_hit(point))
    }

    fn ribbon_hit(&self, point: Point) -> Option<(usize, usize)> {
        self.with_ribbon_bar(|bar| bar.hit_test(point))
    }

    fn close_menu(&mut self) {
        self.menu_open = false;
        self.menu_hover = None;
        self.menu_pinned = None;
        self.recent_hover = None;
    }

    fn toggle_menu(&mut self) {
        self.menu_open = !self.menu_open;
        if self.menu_open {
            self.menu_hover = None;
            self.menu_pinned = None;
            self.recent_hover = None;
            self.set_status_message("Application menu opened");
        } else {
            self.close_menu();
            self.set_status_message("Application menu closed");
        }
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
                let mut msg = String::from("Recent document preview: ");
                msg.push_str(RECENT_DOCS[idx].title);
                self.set_status_message(&msg);
            }
        }
        true
    }

    fn hit_test_editable_item(&self, point: Point) -> Option<usize> {
        let items = self.document_items();
        let canvas = self.document_canvas(items.as_slice());
        let content = canvas.content_rect();
        if !content.contains(point) {
            return None;
        }
        let rel_x = point.x - content.x;
        let rel_y = point.y - content.y;
        for (idx, item) in items.iter().enumerate() {
            if idx != EDITABLE_ITEM_INDEX {
                continue;
            }
            if let DocumentCanvasItem::Text { x, y, text, style } = *item {
                let font = style.font;
                let line_h = font
                    .map(|f| f.line_height())
                    .unwrap_or(sunlight_ui::paint::font::GLYPH_H);
                let max_w = (content.w as i32 - x).max(1) as u32;
                let lines = layout_text_lines(font, text, max_w, line_h);
                if let Some((_line_idx, byte_offset)) =
                    click_to_line_and_byte(font, text, &lines, y, rel_y, rel_x, x)
                {
                    return Some(byte_offset);
                }
            }
        }
        None
    }

    fn editable_item_font(&self) -> Option<&dyn sunlight_ui::VecText> {
        let items = self.document_items();
        if let Some(DocumentCanvasItem::Text { style, .. }) = items.get(EDITABLE_ITEM_INDEX) {
            return style.font;
        }
        None
    }

    fn editable_item_line_h(&self) -> u32 {
        self.editable_item_font()
            .map(|f| f.line_height())
            .unwrap_or(sunlight_ui::paint::font::GLYPH_H)
    }

    fn editable_item_max_w(&self) -> u32 {
        let items = self.document_items();
        let canvas = self.document_canvas(items.as_slice());
        let content = canvas.content_rect();
        let x = if let Some(DocumentCanvasItem::Text { x, .. }) = items.get(EDITABLE_ITEM_INDEX) {
            *x
        } else {
            0
        };
        (content.w as i32 - x).max(1) as u32
    }

    fn compute_edit_lines(&self) -> Vec<TextLineLayout> {
        layout_text_lines(
            self.editable_item_font(),
            self.edit_buffer.as_str(),
            self.editable_item_max_w(),
            self.editable_item_line_h(),
        )
    }

    fn caret_line_index(&self) -> usize {
        let lines = self.compute_edit_lines();
        find_line_index(&lines, self.edit_state.caret_byte).unwrap_or(0)
    }

    fn caret_move_left(&self) -> usize {
        let mut prev = 0usize;
        for (idx, ch) in self.edit_buffer.char_indices() {
            let next = idx + ch.len_utf8();
            if next >= self.edit_state.caret_byte {
                return prev;
            }
            prev = next;
        }
        prev
    }

    fn caret_move_right(&self) -> usize {
        for (idx, _ch) in self.edit_buffer.char_indices() {
            if idx > self.edit_state.caret_byte {
                return idx;
            }
        }
        self.edit_buffer.len()
    }

    fn caret_move_home(&self) -> usize {
        let lines = self.compute_edit_lines();
        let idx = self.caret_line_index();
        line_home_byte(&lines, idx)
    }

    fn caret_move_end(&self) -> usize {
        let lines = self.compute_edit_lines();
        let idx = self.caret_line_index();
        line_end_byte(&lines, idx)
    }

    fn caret_move_up(&self) -> usize {
        let lines = self.compute_edit_lines();
        let cur = self.caret_line_index();
        if cur == 0 {
            return self.edit_state.caret_byte;
        }
        let target_line_idx = cur.saturating_sub(1);
        self.caret_on_line_x(&lines, target_line_idx)
    }

    fn caret_move_down(&self) -> usize {
        let lines = self.compute_edit_lines();
        let cur = self.caret_line_index();
        if cur + 1 >= lines.len() {
            return self.edit_state.caret_byte;
        }
        let target_line_idx = cur + 1;
        self.caret_on_line_x(&lines, target_line_idx)
    }

    fn caret_on_line_x(&self, lines: &[TextLineLayout], line_idx: usize) -> usize {
        let line = match lines.get(line_idx) {
            Some(l) => l,
            None => return self.edit_state.caret_byte,
        };
        let current_preferred_x = self
            .edit_state
            .preferred_caret_x
            .unwrap_or_else(|| {
                let cur_line = lines.get(self.caret_line_index());
                cur_line.map_or(0, |l| {
                    caret_x_on_line(
                        self.editable_item_font(),
                        self.edit_buffer.as_str(),
                        l,
                        self.edit_state.caret_byte,
                    )
                })
            });
        byte_at_x_on_line(
            self.editable_item_font(),
            self.edit_buffer.as_str(),
            line,
            current_preferred_x as i32,
        )
    }

    fn char_before_caret(&self) -> Option<(usize, char)> {
        let mut result = None;
        for (idx, ch) in self.edit_buffer.char_indices() {
            let next = idx + ch.len_utf8();
            if next > self.edit_state.caret_byte {
                return result;
            }
            result = Some((idx, ch));
        }
        result
    }

    fn char_at_caret(&self) -> Option<(usize, char)> {
        self.edit_buffer[self.edit_state.caret_byte..]
            .chars()
            .next()
            .map(|ch| (self.edit_state.caret_byte, ch))
    }

    fn apply_text_mutation(&mut self, new_text: String, new_caret: usize) -> bool {
        self.edit_buffer = new_text;
        self.edit_state.caret_byte = new_caret;
        self.edit_state.selection_anchor_byte = None;
        self.edit_state.preferred_caret_x = None;
        self.document_modified = true;
        true
    }

    fn insert_text_at_caret(&mut self, ch: char) -> bool {
        let mut new_text = String::from(&self.edit_buffer[..self.edit_state.caret_byte]);
        new_text.push(ch);
        new_text.push_str(&self.edit_buffer[self.edit_state.caret_byte..]);
        let new_caret = self.edit_state.caret_byte + ch.len_utf8();
        self.apply_text_mutation(new_text, new_caret)
    }

    fn backspace_at_caret(&mut self) -> bool {
        if let Some((idx, _)) = self.char_before_caret() {
            let mut new_text = String::from(&self.edit_buffer[..idx]);
            new_text.push_str(&self.edit_buffer[self.edit_state.caret_byte..]);
            self.apply_text_mutation(new_text, idx)
        } else {
            false
        }
    }

    fn delete_forward_at_caret(&mut self) -> bool {
        if let Some((_, ch)) = self.char_at_caret() {
            let end = self.edit_state.caret_byte + ch.len_utf8();
            let mut new_text = String::from(&self.edit_buffer[..self.edit_state.caret_byte]);
            new_text.push_str(&self.edit_buffer[end..]);
            self.apply_text_mutation(new_text, self.edit_state.caret_byte)
        } else {
            false
        }
    }

    fn ribbon_action(group_idx: usize, button_idx: usize) -> WriterAction {
        match group_idx {
            0 => FILE_GROUP_DEFS[button_idx].action,
            1 => FONT_GROUP_DEFS[button_idx].action,
            2 => PARAGRAPH_GROUP_DEFS[button_idx].action,
            3 => INSERT_GROUP_DEFS[button_idx].action,
            _ => WriterAction::Open,
        }
    }
}

impl App for WriterApp {
    fn view(&mut self, canvas: &mut sunlight_ui::Canvas, theme: &Theme) {
        let document_items = self.document_items();
        let document_canvas = self.document_canvas(document_items.as_slice());
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);
        self.with_header(|header| header.draw(canvas, theme));
        self.with_ribbon_bar(|bar| bar.draw(canvas, theme));
        document_canvas.draw(canvas, theme);
        let right_status = if self.document_modified {
            "100% | Modified"
        } else {
            "100% | Document Canvas Active | Editable"
        };
        StatusBar::new(
            self.status_rect(),
            "Page 1 of 1 | Col 1",
            self.status_center.as_str(),
            right_status,
        )
        .draw(canvas, theme);
        if self.menu_open {
            self.with_app_menu(|menu| menu.draw(canvas, theme));
        }
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Tick => {
                if self.status_ticks > 0 {
                    self.status_ticks -= 1;
                    if self.status_ticks == 0 {
                        self.status_center.clear();
                        self.status_center.set("Document Canvas Ready");
                        return true;
                    }
                }
                false
            }
            Event::MouseMove { x, y } => {
                let point = Point::new(x, y);
                let mut redraw = false;

                let menu_button_hover = self.menu_button_hit(point);
                if menu_button_hover != self.menu_button_hover {
                    self.menu_button_hover = menu_button_hover;
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
                    let menu_hover = self.menu_command_hit(point);
                    let recent_hover = self.recent_doc_hit(point);
                    if menu_hover != self.menu_hover || recent_hover != self.recent_hover {
                        self.menu_hover = menu_hover;
                        self.recent_hover = recent_hover;
                        redraw = true;
                    }
                    // When the app menu is open, document cursor semantics
                    // are suppressed — the menu overlay dominates.
                    let cursor = CursorShape::Pointer;
                    if cursor != self.prev_document_cursor {
                        set_client_cursor(cursor);
                        self.prev_document_cursor = cursor;
                    }
                } else {
                    let items = self.document_items();
                    let canvas = self.document_canvas(items.as_slice());
                    let target = canvas.hit_target(point);
                    let cursor = match target {
                        CanvasHitTarget::Link => CursorShape::Hand,
                        CanvasHitTarget::Text => CursorShape::Text,
                        CanvasHitTarget::None => CursorShape::Pointer,
                    };
                    if cursor != self.prev_document_cursor {
                        set_client_cursor(cursor);
                        self.prev_document_cursor = cursor;
                    }
                }

                redraw
            }
            Event::Click { x, y } => {
                let point = Point::new(x, y);

                if self.menu_button_hit(point) {
                    self.toggle_menu();
                    return true;
                }

                if self.menu_open {
                    if let Some(idx) = self.recent_doc_hit(point) {
                        self.close_menu();
                        return self.dispatch_action(WriterAction::RecentDocument(idx));
                    }

                    if let Some(idx) = self.menu_command_hit(point) {
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

                if let Some((group_idx, button_idx)) = self.ribbon_hit(point) {
                    return self.dispatch_action(Self::ribbon_action(group_idx, button_idx));
                }

                let hit = self.hit_test_editable_item(point);
                if let Some(byte_offset) = hit {
                    self.edit_state.active_item_index = Some(EDITABLE_ITEM_INDEX);
                    self.edit_state.caret_byte = byte_offset;
                    self.edit_state.selection_anchor_byte = None;
                    self.edit_state.preferred_caret_x = None;
                    return true;
                }

                if self.edit_state.is_editing() {
                    self.edit_state.clear();
                    return true;
                }

                false
            }
            Event::Key(ch) if self.edit_state.is_editing() => {
                if ch == '\u{8}' {
                    return self.backspace_at_caret();
                }
                if ch == '\r' {
                    return false;
                }
                if ch == '\t' || ch == '\u{1b}' {
                    return false;
                }
                if ch.is_control() && ch != '\n' {
                    return false;
                }
                self.insert_text_at_caret(ch)
            }
            Event::KeyPress {
                keycode, pressed, ..
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
                    if self.edit_state.is_editing() {
                        self.edit_state.clear();
                        return true;
                    }
                    request_close();
                }
                if self.edit_state.is_editing() {
                    match keycode {
                        KEY_LEFT => {
                            let new_caret = self.caret_move_left();
                            if new_caret != self.edit_state.caret_byte {
                                self.edit_state.caret_byte = new_caret;
                                self.edit_state.selection_anchor_byte = None;
                                self.edit_state.preferred_caret_x = None;
                                return true;
                            }
                        }
                        KEY_RIGHT => {
                            let new_caret = self.caret_move_right();
                            if new_caret != self.edit_state.caret_byte {
                                self.edit_state.caret_byte = new_caret;
                                self.edit_state.selection_anchor_byte = None;
                                self.edit_state.preferred_caret_x = None;
                                return true;
                            }
                        }
                        KEY_UP => {
                            let new_caret = self.caret_move_up();
                            if new_caret != self.edit_state.caret_byte {
                                let old_x = self.edit_state.preferred_caret_x;
                                self.edit_state.caret_byte = new_caret;
                                self.edit_state.selection_anchor_byte = None;
                                self.edit_state.preferred_caret_x = old_x
                                    .or_else(|| {
                                        let lines = self.compute_edit_lines();
                                        let cur = self.caret_line_index();
                                        lines.get(cur).map(|l| {
                                            caret_x_on_line(
                                                self.editable_item_font(),
                                                self.edit_buffer.as_str(),
                                                l,
                                                new_caret,
                                            )
                                        })
                                    });
                                return true;
                            }
                        }
                        KEY_DOWN => {
                            let new_caret = self.caret_move_down();
                            if new_caret != self.edit_state.caret_byte {
                                let old_x = self.edit_state.preferred_caret_x;
                                self.edit_state.caret_byte = new_caret;
                                self.edit_state.selection_anchor_byte = None;
                                self.edit_state.preferred_caret_x = old_x
                                    .or_else(|| {
                                        let lines = self.compute_edit_lines();
                                        let cur = self.caret_line_index();
                                        lines.get(cur).map(|l| {
                                            caret_x_on_line(
                                                self.editable_item_font(),
                                                self.edit_buffer.as_str(),
                                                l,
                                                new_caret,
                                            )
                                        })
                                    });
                                return true;
                            }
                        }
                        KEY_HOME => {
                            let new_caret = self.caret_move_home();
                            if new_caret != self.edit_state.caret_byte {
                                self.edit_state.caret_byte = new_caret;
                                self.edit_state.selection_anchor_byte = None;
                                self.edit_state.preferred_caret_x = None;
                                return true;
                            }
                        }
                        KEY_END => {
                            let end = self.caret_move_end();
                            if self.edit_state.caret_byte != end {
                                self.edit_state.caret_byte = end;
                                self.edit_state.selection_anchor_byte = None;
                                self.edit_state.preferred_caret_x = None;
                                return true;
                            }
                        }
                        KEY_DELETE => {
                            return self.delete_forward_at_caret();
                        }
                        _ => {}
                    }
                }
                false
            }
            _ => false,
        }
    }
}

#[cfg(not(test))]
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

#[cfg(test)]
mod tests {
    use super::{DocumentCanvasItem, DocumentCanvasMode, WriterDocument};

    #[test]
    fn sample_document_converts_to_non_empty_canvas_items() {
        let items = WriterDocument::sample().to_canvas_items();
        assert!(!items.is_empty());
    }

    #[test]
    fn sample_document_contains_link_item() {
        let items = WriterDocument::sample().to_canvas_items();
        assert!(items
            .iter()
            .any(|item| matches!(item, DocumentCanvasItem::LinkText { .. })));
    }

    #[test]
    fn sample_document_keeps_editable_unicode_emoji_text() {
        let document = WriterDocument::sample();
        assert_eq!(document.mode, DocumentCanvasMode::Editable);
        assert!(document.blocks.iter().any(|block| {
            matches!(block, super::WriterBlock::Text { text, .. } if text.contains("🐇") && text.contains("🦀"))
        }));
    }

    #[test]
    fn empty_document_converts_without_panic() {
        let items = WriterDocument::empty(DocumentCanvasMode::Editable).to_canvas_items();
        assert!(items.is_empty());
    }
}
