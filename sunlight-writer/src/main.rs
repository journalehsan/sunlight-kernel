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
    AppMenuCommand, AppMenuSecondaryItem, DocumentCanvas, DocumentCanvasItem, DocumentCanvasMode,
    DocumentCanvasPresentation, DocumentEditor, DocumentRectStyle, DocumentStrokeStyle,
    DocumentTextStyle, FormattingState, HeaderActionButton, HeaderChip, PremiumHeader, RibbonBar,
    RibbonButtonKind, RibbonButtonSpec, RibbonGroupSpec, RichTextFonts, StatusBar, StyleProperty,
    TwoPaneAppMenu,
};
use sunlight_ui::{
    request_close, set_client_cursor, App, AxisSizing, Color, Column, CursorShape, Event,
    LayoutBox, LayoutInvalidation, Point, Rect, Size, Sizing, Theme, VecText, Window, WindowConfig,
    WindowDecoration, WindowEvent,
};

mod persistence;
use persistence::{
    export, format_from_path, import, loses_formatting, DocumentFormat, WriterDocumentSession,
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
const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
const KEY_ESC: u8 = 0x01;

const KEY_LEFT: u8 = 0x4B;
const KEY_RIGHT: u8 = 0x4D;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_HOME: u8 = 0x47;
const KEY_END: u8 = 0x4F;
const KEY_DELETE: u8 = 0x53;
const KEY_PAGE_UP: u8 = 0x49;
const KEY_PAGE_DOWN: u8 = 0x51;
const KEY_A: u8 = 0x1E;
const KEY_O: u8 = 0x18;
const KEY_S: u8 = 0x1F;
const KEY_C: u8 = 0x2E;
const KEY_V: u8 = 0x2F;
const KEY_X: u8 = 0x2D;
const KEY_B: u8 = 0x30;
const KEY_I: u8 = 0x17;
const KEY_U: u8 = 0x16;

const EDITABLE_ITEM_INDEX: usize = 0;
const WHEEL_SCROLL_LINES: i32 = 3;

static FONT_UI_TITLE: VecFont = VecFont(FontRole::UiTitle);
static FONT_UI_LARGE: VecFont = VecFont(FontRole::UiLarge);
static FONT_UI_MEDIUM: VecFont = VecFont(FontRole::UiMedium);
static FONT_UI_BOLD: VecFont = VecFont(FontRole::UiBold);
static FONT_UI_ITALIC: VecFont = VecFont(FontRole::UiItalic);
static FONT_UI_BOLD_ITALIC: VecFont = VecFont(FontRole::UiBoldItalic);
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

fn show_writer_dialog(
    request: &sunlight_dialogs::DialogRequest,
) -> Result<sunlight_dialogs::DialogResult, &'static str> {
    let result = sunlight_dialogs::DialogClient::new()
        .show(request)
        .map_err(|_| "Dialog host unavailable")?;
    Ok(result)
}

fn read_writer_file(path: &[u8]) -> Result<Vec<u8>, &'static str> {
    let fd = sunlight_libc::open(path).map_err(|_| "Could not open file")?;
    let mut out = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    loop {
        let count = sunlight_libc::read(fd, &mut chunk).map_err(|_| "Read failed")?;
        if count == 0 {
            break;
        }
        if out.len().saturating_add(count) > MAX_FILE_BYTES {
            let _ = sunlight_libc::close(fd);
            return Err("File too large");
        }
        out.extend_from_slice(&chunk[..count]);
    }
    let _ = sunlight_libc::close(fd);
    Ok(out)
}

fn push_decimal(out: &mut String, mut value: u64) {
    if value == 0 {
        out.push('0');
        return;
    }
    let mut digits = [0u8; 20];
    let mut len = 0;
    while value > 0 {
        digits[len] = b'0' + (value % 10) as u8;
        len += 1;
        value /= 10;
    }
    while len > 0 {
        len -= 1;
        out.push(digits[len] as char);
    }
}

fn write_writer_file(path: &[u8], data: &[u8]) -> Result<(), &'static str> {
    if data.len() > MAX_FILE_BYTES {
        return Err("Document is too large to save");
    }
    let path_str = core::str::from_utf8(path).map_err(|_| "Invalid path")?;
    let mut temporary = String::from(path_str);
    temporary.push_str(".sunwriter-");
    push_decimal(&mut temporary, sunlight_ipc::getpid());
    let temp_bytes = temporary.as_bytes();
    let fd = sunlight_libc::open_with_flags(
        temp_bytes,
        sunlight_libc::O_WRONLY | sunlight_libc::O_CREAT | sunlight_libc::O_TRUNC,
    )
    .map_err(|_| "Could not create temporary file")?;
    let mut offset = 0;
    while offset < data.len() {
        let count = sunlight_libc::write(fd, &data[offset..]).map_err(|_| "Write failed")?;
        if count == 0 {
            let _ = sunlight_libc::close(fd);
            let _ = sunlight_libc::unlink(temp_bytes);
            return Err("Write stalled");
        }
        offset += count;
    }
    sunlight_libc::close(fd).map_err(|_| "Could not close temporary file")?;
    if sunlight_libc::rename(temp_bytes, path).is_ok() {
        return Ok(());
    }
    let _ = sunlight_libc::unlink(temp_bytes);
    let fd = sunlight_libc::open_with_flags(
        path,
        sunlight_libc::O_WRONLY | sunlight_libc::O_CREAT | sunlight_libc::O_TRUNC,
    )
    .map_err(|_| "Could not open destination for writing")?;
    let mut offset = 0;
    while offset < data.len() {
        let count = sunlight_libc::write(fd, &data[offset..]).map_err(|_| "Write failed")?;
        if count == 0 {
            let _ = sunlight_libc::close(fd);
            return Err("Write stalled");
        }
        offset += count;
    }
    sunlight_libc::close(fd).map_err(|_| "Could not close destination")
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
    window_title: TextSlot,
    status_ticks: u16,
    editor: DocumentEditor,
    editor_focused: bool,
    drag_anchor_byte: Option<usize>,
    session: WriterDocumentSession,
    prev_document_cursor: CursorShape,
    client_bounds: Rect,
    layout_invalidation: LayoutInvalidation,
    layout: WriterLayout,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct WriterLayout {
    root: Rect,
    header: Rect,
    ribbon: Rect,
    workspace: Rect,
    status: Rect,
}

impl WriterApp {
    fn new() -> Self {
        let mut status_center = TextSlot::empty();
        status_center.set("Document Canvas Ready");
        let mut window_title = TextSlot::empty();
        window_title.set("Untitled — Sunlight Writer");
        let mut app = Self {
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
            window_title,
            status_ticks: 0,
            editor: DocumentEditor::new(),
            editor_focused: false,
            drag_anchor_byte: None,
            session: WriterDocumentSession::new(),
            prev_document_cursor: CursorShape::Pointer,
            client_bounds: Rect::new(0, 0, WIN_W, WIN_H),
            layout_invalidation: LayoutInvalidation::new(),
            layout: WriterLayout::default(),
        };
        let _ = app.ensure_layout();
        let _ = app.configure_editor_layout();
        app
    }

    fn compute_layout(root: Rect) -> WriterLayout {
        let fixed_height = |height| {
            LayoutBox::new(Rect::new(0, 0, 0, height))
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fixed(height)))
        };
        let mut children = [
            fixed_height(TOP_BAR_H),
            fixed_height(RIBBON_H),
            LayoutBox::new(Rect::default())
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fill)),
            fixed_height(STATUS_H),
        ];
        let _ = Column::new(root).arrange(&mut children);
        WriterLayout {
            root,
            header: children[0].bounds(),
            ribbon: children[1].bounds(),
            workspace: children[2].bounds(),
            status: children[3].bounds(),
        }
    }

    fn ensure_layout(&mut self) -> bool {
        if !self.layout_invalidation.update(self.client_bounds) {
            return false;
        }
        self.layout = Self::compute_layout(self.client_bounds);
        true
    }

    fn set_client_bounds(&mut self, width: u32, height: u32) -> bool {
        let bounds = Rect::new(0, 0, width, height);
        if bounds == self.client_bounds {
            return false;
        }
        self.client_bounds = bounds;
        self.layout_invalidation.invalidate();
        self.ensure_layout()
    }

    fn set_status_message(&mut self, text: &str) {
        self.status_center.set(text);
        self.status_ticks = 32;
    }

    fn sync_window_title(&mut self) {
        let mut title = String::new();
        if self.session.is_dirty() {
            title.push('*');
        }
        if let Some(path) = self.session.path.as_deref() {
            title.push_str(path.rsplit('/').next().unwrap_or(path));
        } else {
            title.push_str("Untitled");
        }
        title.push_str(" — Sunlight Writer");
        self.window_title.set(&title);
    }

    fn top_bar_rect(&self) -> Rect {
        self.layout.header
    }

    fn ribbon_rect(&self) -> Rect {
        self.layout.ribbon
    }

    fn content_rect(&self) -> Rect {
        self.layout.workspace
    }

    fn status_rect(&self) -> Rect {
        self.layout.status
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
            title: self.window_title.as_str(),
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
        let mut active = [(0, 0); 3];
        let mut mixed = [(0, 0); 3];
        let mut active_len = 0;
        let mut mixed_len = 0;
        for (button, property) in [
            (2, StyleProperty::Bold),
            (3, StyleProperty::Italic),
            (4, StyleProperty::Underline),
        ] {
            match self.editor.formatting_state(property) {
                FormattingState::On => {
                    active[active_len] = (1, button);
                    active_len += 1;
                }
                FormattingState::Mixed => {
                    mixed[mixed_len] = (1, button);
                    mixed_len += 1;
                }
                FormattingState::Off => {}
            }
        }
        f(RibbonBar {
            rect: self.ribbon_rect(),
            groups: &groups,
            hovered: self.ribbon_hover,
            active: &active[..active_len],
            mixed: &mixed[..mixed_len],
            label_font: Some(&FONT_UI_REGULAR),
            small_font: Some(&FONT_UI_SMALL),
        })
    }

    fn document_items(&self) -> Vec<DocumentCanvasItem<'_>> {
        Vec::from([DocumentCanvasItem::Text {
            x: 0,
            y: 0,
            text: self.editor.text(),
            style: writer_text_style(WriterTextRole::Paragraph),
        }])
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
            .with_document_editor(EDITABLE_ITEM_INDEX, &self.editor)
            .with_rich_text_fonts(Self::rich_text_fonts())
            .with_caret_visible(self.editor_focused)
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
            WriterAction::New => {
                if !self.confirm_replace("Create a new document?") {
                    return true;
                }
                self.editor
                    .set_document(sunlight_ui::widgets::RichDocument::new());
                self.session = WriterDocumentSession::new();
                self.sync_window_title();
                self.editor_focused = true;
                let _ = self.configure_editor_layout();
                self.set_status_message("New document");
            }
            WriterAction::Open => return self.open_document(),
            WriterAction::Save => return self.save_document(false),
            WriterAction::SaveAs => return self.save_document(true),
            WriterAction::Print => self.set_status_message("Print is a placeholder command"),
            WriterAction::Share => self.set_status_message("Share is a placeholder command"),
            WriterAction::Export => self.set_status_message("Export is a placeholder command"),
            WriterAction::Exit => return self.try_close(),
            WriterAction::FontFamily => self.set_status_message("Font picker is visual only"),
            WriterAction::FontSize => self.set_status_message("Font size picker is visual only"),
            WriterAction::Bold => return self.apply_format(StyleProperty::Bold, "Bold"),
            WriterAction::Italic => return self.apply_format(StyleProperty::Italic, "Italic"),
            WriterAction::Underline => {
                return self.apply_format(StyleProperty::Underline, "Underline")
            }
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

    fn editor_content_rect(&self) -> Rect {
        DocumentCanvas::new(self.content_rect(), &[])
            .with_presentation(DocumentCanvasPresentation::Writer)
            .content_rect()
    }

    fn rich_text_fonts() -> RichTextFonts<'static> {
        RichTextFonts {
            regular: Some(&FONT_UI_MEDIUM),
            bold: Some(&FONT_UI_BOLD),
            italic: Some(&FONT_UI_ITALIC),
            bold_italic: Some(&FONT_UI_BOLD_ITALIC),
        }
    }

    fn apply_format(&mut self, property: StyleProperty, label: &str) -> bool {
        let changed = self.editor.toggle_format(property);
        if changed {
            self.session.mark_changed(self.editor.document());
            self.sync_window_title();
            let _ = self.configure_editor_layout();
            self.editor_focused = true;
            let mut message = String::from(label);
            message.push_str(" formatting updated");
            self.set_status_message(&message);
        }
        changed
    }

    fn configure_editor_layout(&mut self) -> bool {
        let content = self.editor_content_rect();
        self.editor.configure_rich_layout(
            Self::rich_text_fonts(),
            content.w,
            content.h,
            FONT_UI_MEDIUM.line_height(),
        )
    }

    fn editor_hit_test(&self, point: Point) -> Option<usize> {
        let content = self.editor_content_rect();
        content.contains(point).then(|| {
            self.editor.rich_hit_test(
                Self::rich_text_fonts(),
                point.x - content.x,
                point.y - content.y,
            )
        })
    }

    fn apply_editor_change(&mut self, changed: bool) -> bool {
        if changed {
            self.session.mark_changed(self.editor.document());
            self.sync_window_title();
            let _ = self.configure_editor_layout();
        }
        changed
    }

    fn copy_selection(&mut self) -> bool {
        let Some(text) = self.editor.selected_text() else {
            return false;
        };
        match sunlight_ui::clipboard::set_text_from(b"sunlight-writer", text) {
            Ok(()) => true,
            Err(error) => {
                self.set_status_message(error.message());
                true
            }
        }
    }

    fn cut_selection(&mut self) -> bool {
        let Some(text) = self.editor.selected_text() else {
            return false;
        };
        match sunlight_ui::clipboard::set_text_from(b"sunlight-writer", text) {
            Ok(()) => {
                let changed = self.editor.delete_selection();
                self.apply_editor_change(changed)
            }
            Err(error) => {
                self.set_status_message(error.message());
                true
            }
        }
    }

    fn paste_clipboard(&mut self) -> bool {
        match sunlight_ui::clipboard::get_text() {
            Ok(text) => {
                let changed = self.editor.insert_str(&text);
                self.apply_editor_change(changed)
            }
            Err(error) => {
                self.set_status_message(error.message());
                true
            }
        }
    }

    fn confirm_replace(&mut self, message: &str) -> bool {
        if !self.session.is_dirty() {
            return true;
        }
        match show_writer_dialog(&sunlight_dialogs::DialogRequest::Confirm(
            sunlight_dialogs::ConfirmRequest {
                common: sunlight_dialogs::DialogCommonOptions {
                    title: String::from("Unsaved Changes"),
                    message: String::from(message),
                    severity: sunlight_dialogs::DialogSeverity::Question,
                    silent: false,
                },
                style: sunlight_dialogs::ConfirmStyle::YesNo,
                default_button: sunlight_dialogs::DialogButton::Yes,
            },
        )) {
            Ok(sunlight_dialogs::DialogResult::Yes) => self.save_document(false),
            Ok(sunlight_dialogs::DialogResult::No) => true,
            Ok(
                sunlight_dialogs::DialogResult::Cancel
                | sunlight_dialogs::DialogResult::Cancelled
                | sunlight_dialogs::DialogResult::Dismissed,
            ) => false,
            Ok(sunlight_dialogs::DialogResult::Error(message)) => {
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

    fn open_document(&mut self) -> bool {
        if !self.confirm_replace("Open another document and discard unsaved changes?") {
            return true;
        }
        let request =
            sunlight_dialogs::DialogRequest::OpenFile(sunlight_dialogs::OpenFileRequest {
                title: String::from("Open Document"),
                initial_dir: Some(String::from("/root")),
                allowed_mime_types: Vec::from([
                    String::from("text/markdown"),
                    String::from("text/plain"),
                ]),
                allowed_extensions: Vec::from([
                    String::from(".md"),
                    String::from(".markdown"),
                    String::from(".txt"),
                ]),
                allow_multiple: false,
                show_preview: true,
                confirm_button_label: Some(String::from("Open")),
            });
        match show_writer_dialog(&request) {
            Ok(sunlight_dialogs::DialogResult::FileSelected(path)) => {
                let Some(format) = format_from_path(&path) else {
                    self.set_status_message("Choose a Markdown or text file");
                    return true;
                };
                match read_writer_file(path.as_bytes()) {
                    Ok(bytes) => match import(format, &bytes) {
                        Ok(document) => {
                            self.editor.set_document(document.clone());
                            self.session.replace_loaded(document, path, format);
                            self.sync_window_title();
                            self.editor_focused = true;
                            let _ = self.configure_editor_layout();
                            self.set_status_message("Document opened");
                        }
                        Err(error) => self.set_status_message(error.message()),
                    },
                    Err(message) => self.set_status_message(message),
                }
            }
            Ok(
                sunlight_dialogs::DialogResult::Cancelled
                | sunlight_dialogs::DialogResult::Cancel
                | sunlight_dialogs::DialogResult::Dismissed,
            ) => self.set_status_message("Open cancelled"),
            Ok(sunlight_dialogs::DialogResult::Error(message)) => self.set_status_message(&message),
            Err(message) => self.set_status_message(message),
            _ => self.set_status_message("Open dialog returned an unexpected result"),
        }
        true
    }

    fn open_path(&mut self, path: String) {
        let Some(format) = format_from_path(&path) else {
            self.set_status_message("Choose a Markdown or text file");
            return;
        };
        match read_writer_file(path.as_bytes()) {
            Ok(bytes) => match import(format, &bytes) {
                Ok(document) => {
                    self.editor.set_document(document.clone());
                    self.session.replace_loaded(document, path, format);
                    self.sync_window_title();
                    let _ = self.configure_editor_layout();
                    self.set_status_message("Document opened");
                }
                Err(error) => self.set_status_message(error.message()),
            },
            Err(message) => self.set_status_message(message),
        }
    }

    fn save_document(&mut self, force_save_as: bool) -> bool {
        let target = if !force_save_as {
            self.session.path.clone().zip(self.session.format)
        } else {
            None
        };
        let (path, format) = if let Some(target) = target {
            target
        } else {
            let request =
                sunlight_dialogs::DialogRequest::SaveFile(sunlight_dialogs::SaveFileRequest {
                    title: String::from("Save Document As"),
                    initial_dir: Some(String::from("/root")),
                    suggested_name: Some(String::from("untitled.md")),
                    default_extension: Some(String::from(".md")),
                    allowed_extensions: Vec::from([
                        String::from(".md"),
                        String::from(".markdown"),
                        String::from(".txt"),
                    ]),
                    overwrite_confirm: true,
                    confirm_button_label: Some(String::from("Save")),
                });
            let selected = match show_writer_dialog(&request) {
                Ok(sunlight_dialogs::DialogResult::SavePathSelected(path)) => path,
                Ok(
                    sunlight_dialogs::DialogResult::Cancelled
                    | sunlight_dialogs::DialogResult::Cancel
                    | sunlight_dialogs::DialogResult::Dismissed,
                ) => {
                    self.set_status_message("Save cancelled");
                    return false;
                }
                Ok(sunlight_dialogs::DialogResult::Error(message)) => {
                    self.set_status_message(&message);
                    return false;
                }
                Err(message) => {
                    self.set_status_message(message);
                    return false;
                }
                _ => {
                    self.set_status_message("Save dialog returned an unexpected result");
                    return false;
                }
            };
            let format = format_from_path(&selected).unwrap_or(DocumentFormat::Markdown);
            (selected, format)
        };
        if force_save_as && loses_formatting(format, self.editor.document()) {
            let message = match format {
                DocumentFormat::PlainText => {
                    "Some formatting cannot be saved in plain text. Continue?"
                }
                DocumentFormat::Markdown => {
                    "Underline formatting cannot be saved in Markdown. Continue?"
                }
            };
            let request =
                sunlight_dialogs::DialogRequest::Confirm(sunlight_dialogs::ConfirmRequest {
                    common: sunlight_dialogs::DialogCommonOptions {
                        title: String::from("Formatting Will Be Lost"),
                        message: String::from(message),
                        severity: sunlight_dialogs::DialogSeverity::Warning,
                        silent: false,
                    },
                    style: sunlight_dialogs::ConfirmStyle::YesNo,
                    default_button: sunlight_dialogs::DialogButton::No,
                });
            if !matches!(
                show_writer_dialog(&request),
                Ok(sunlight_dialogs::DialogResult::Yes)
            ) {
                self.set_status_message("Save cancelled");
                return false;
            }
        }
        let data = export(format, self.editor.document());
        match write_writer_file(path.as_bytes(), data.as_bytes()) {
            Ok(()) => {
                self.session.set_saved_target(path, format);
                self.session.mark_saved(self.editor.document());
                self.sync_window_title();
                self.set_status_message("Document saved");
                true
            }
            Err(message) => {
                self.set_status_message(message);
                false
            }
        }
    }

    fn try_close(&mut self) -> bool {
        if self.session.is_dirty() {
            if !self.confirm_replace("Save changes before closing?") {
                return true;
            }
        }
        request_close();
        true
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
        if self.client_bounds.size() != Size::new(canvas.width, canvas.height) {
            let _ = self.set_client_bounds(canvas.width, canvas.height);
        } else {
            let _ = self.ensure_layout();
        }
        let _ = self.configure_editor_layout();
        let document_items = self.document_items();
        let document_canvas = self.document_canvas(document_items.as_slice());
        canvas.fill_rect(self.layout.root, theme.bg);
        self.with_header(|header| header.draw(canvas, theme));
        self.with_ribbon_bar(|bar| bar.draw(canvas, theme));
        document_canvas.draw(canvas, theme);
        let right_status = if self.session.is_dirty() {
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
                    let cursor = if self.editor_content_rect().contains(point) {
                        CursorShape::Text
                    } else {
                        CursorShape::Pointer
                    };
                    if cursor != self.prev_document_cursor {
                        set_client_cursor(cursor);
                        self.prev_document_cursor = cursor;
                    }
                }

                if let Some(anchor) = self.drag_anchor_byte {
                    let content = self.editor_content_rect();
                    let local_x = x - content.x;
                    let local_y = y - content.y;
                    if y < content.y {
                        let _ = self.editor.scroll_by(-(self.editor.line_height() as i32));
                    } else if y >= content.bottom() {
                        let _ = self.editor.scroll_by(self.editor.line_height() as i32);
                    }
                    if self.editor.rich_pointer_select(
                        Self::rich_text_fonts(),
                        local_x,
                        local_y,
                        Some(anchor),
                    ) {
                        redraw = true;
                    }
                }

                redraw
            }
            Event::MouseDown { x, y, button: 0 } => {
                let point = Point::new(x, y);
                let Some(byte) = self.editor_hit_test(point) else {
                    return false;
                };
                self.editor_focused = true;
                self.drag_anchor_byte = Some(byte);
                self.editor.rich_pointer_select(
                    Self::rich_text_fonts(),
                    x - self.editor_content_rect().x,
                    y - self.editor_content_rect().y,
                    None,
                );
                true
            }
            Event::MouseUp { button: 0, .. } => {
                let was_dragging = self.drag_anchor_byte.take().is_some();
                was_dragging
            }
            Event::MouseWheel { x, y, delta } => {
                if !self.editor_content_rect().contains(Point::new(x, y)) || delta == 0 {
                    return false;
                }
                let detents = if delta.unsigned_abs() >= 120 {
                    delta as i32 / 120
                } else {
                    delta.signum() as i32
                };
                self.editor.scroll_by(
                    detents
                        .saturating_mul(WHEEL_SCROLL_LINES)
                        .saturating_mul(self.editor.line_height() as i32),
                )
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

                if self.editor_content_rect().contains(point) {
                    self.editor_focused = true;
                    return true;
                }

                if self.editor_focused {
                    self.editor_focused = false;
                    return true;
                }

                false
            }
            Event::Key(ch) if self.editor_focused => {
                if ch == '\u{8}' {
                    let changed = self.editor.backspace();
                    return self.apply_editor_change(changed);
                }
                if ch == '\r' || ch == '\n' {
                    let changed = self.editor.insert_newline();
                    return self.apply_editor_change(changed);
                }
                if ch == '\t' || ch == '\u{1b}' {
                    return false;
                }
                if ch.is_control() {
                    return false;
                }
                let changed = self.editor.insert_char(ch);
                self.apply_editor_change(changed)
            }
            Event::KeyPress {
                keycode,
                pressed,
                shift,
                ctrl,
                alt,
                super_key,
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
                    if self.editor_focused {
                        self.editor_focused = false;
                        return true;
                    }
                    request_close();
                }
                if ctrl && !alt && !super_key {
                    match keycode {
                        KEY_O => return self.dispatch_action(WriterAction::Open),
                        KEY_S if shift => return self.dispatch_action(WriterAction::SaveAs),
                        KEY_S => return self.dispatch_action(WriterAction::Save),
                        _ => {}
                    }
                }
                if self.editor_focused && !alt && !super_key {
                    if ctrl {
                        return match keycode {
                            KEY_A => self.editor.select_all(),
                            KEY_C => self.copy_selection(),
                            KEY_X => self.cut_selection(),
                            KEY_V => self.paste_clipboard(),
                            KEY_B => self.dispatch_action(WriterAction::Bold),
                            KEY_I => self.dispatch_action(WriterAction::Italic),
                            KEY_U => self.dispatch_action(WriterAction::Underline),
                            _ => false,
                        };
                    }
                    let changed = match keycode {
                        KEY_LEFT => self.editor.move_left(shift),
                        KEY_RIGHT => self.editor.move_right(shift),
                        KEY_UP => self.editor.move_up_rich(Self::rich_text_fonts(), shift),
                        KEY_DOWN => self.editor.move_down_rich(Self::rich_text_fonts(), shift),
                        KEY_HOME => self.editor.move_home(shift),
                        KEY_END => self.editor.move_end(shift),
                        KEY_PAGE_UP => self.editor.page_up_rich(Self::rich_text_fonts(), shift),
                        KEY_PAGE_DOWN => self.editor.page_down_rich(Self::rich_text_fonts(), shift),
                        KEY_DELETE => {
                            let changed = self.editor.delete_forward();
                            return self.apply_editor_change(changed);
                        }
                        _ => false,
                    };
                    return changed;
                }
                false
            }
            _ => false,
        }
    }

    fn window_event(&mut self, event: WindowEvent) -> bool {
        let WindowEvent::Resized { width, height } = event;
        let changed = self.set_client_bounds(width, height);
        let editor_changed = self.configure_editor_layout();
        changed || editor_changed
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
    if argc > 1 && !argv.is_null() {
        let raw = unsafe { *argv.add(1) };
        if !raw.is_null() {
            let len = unsafe { sunlight_libc::crt0::cstr_len(raw, sunlight_libc::MAX_PATH) };
            if len > 0 {
                let bytes = unsafe { core::slice::from_raw_parts(raw, len) };
                if let Ok(path) = core::str::from_utf8(bytes) {
                    app.open_path(String::from(path));
                }
            }
        }
    }
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
    use super::{
        DocumentCanvasItem, DocumentCanvasMode, Rect, WriterAction, WriterApp, WriterDocument,
        KEY_B, KEY_I, KEY_U, RIBBON_H, STATUS_H, TOP_BAR_H,
    };
    use alloc::string::String;
    use sunlight_ui::widgets::{FormattingState, StyleProperty};
    use sunlight_ui::{App, Event};

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

    #[test]
    fn responsive_chrome_and_workspace_follow_client_bounds() {
        let mut app = WriterApp::new();
        assert!(app.set_client_bounds(900, 700));
        assert_eq!(app.top_bar_rect(), Rect::new(0, 0, 900, TOP_BAR_H));
        assert_eq!(
            app.ribbon_rect(),
            Rect::new(0, TOP_BAR_H as i32, 900, RIBBON_H)
        );
        assert_eq!(
            app.content_rect(),
            Rect::new(
                0,
                (TOP_BAR_H + RIBBON_H) as i32,
                900,
                700 - TOP_BAR_H - RIBBON_H - STATUS_H,
            )
        );
        assert_eq!(app.status_rect(), Rect::new(0, 678, 900, STATUS_H));
    }

    #[test]
    fn document_canvas_receives_current_workspace() {
        let mut app = WriterApp::new();
        let _ = app.set_client_bounds(780, 610);
        let items = app.document_items();
        let canvas = app.document_canvas(items.as_slice());
        assert_eq!(canvas.rect, app.content_rect());
    }

    #[test]
    fn writer_format_commands_preserve_selection_and_update_toolbar_state() {
        let mut app = WriterApp::new();
        app.editor.insert_str("hello world");
        app.editor.set_caret(0, false);
        app.editor.set_caret(5, true);
        app.editor_focused = true;
        assert!(app.dispatch_action(WriterAction::Bold));
        assert_eq!(app.editor.selected_text(), Some("hello"));
        assert_eq!(
            app.editor.formatting_state(StyleProperty::Bold),
            FormattingState::On
        );
        assert!(app.editor_focused);
        app.with_ribbon_bar(|bar| assert!(bar.active.contains(&(1, 2))));
        app.editor.select_all();
        app.with_ribbon_bar(|bar| assert!(bar.mixed.contains(&(1, 2))));
    }

    #[test]
    fn writer_ctrl_shortcuts_toggle_composable_typing_styles() {
        let mut app = WriterApp::new();
        app.editor_focused = true;
        for keycode in [KEY_B, KEY_I, KEY_U] {
            assert!(app.update(Event::KeyPress {
                keycode,
                pressed: true,
                shift: false,
                ctrl: true,
                alt: false,
                super_key: false
            }));
        }
        app.update(Event::Key('x'));
        let style = app.editor.document().runs()[0].style;
        assert!(style.bold && style.italic && style.underline);
    }

    #[test]
    fn wider_workspace_recenters_without_stretching_writer_page() {
        let mut app = WriterApp::new();
        let initial_items = app.document_items();
        let initial_page = app.document_canvas(initial_items.as_slice()).page_rect();
        let initial_item_count = initial_items.len();
        drop(initial_items);

        let _ = app.set_client_bounds(1600, 860);
        let wide_items = app.document_items();
        let wide_page = app.document_canvas(wide_items.as_slice()).page_rect();

        assert_eq!(initial_page.w, 860);
        assert_eq!(wide_page.w, initial_page.w);
        assert!(wide_page.x > initial_page.x);
        assert_eq!(wide_items.len(), initial_item_count);
    }

    #[test]
    fn tiny_and_zero_workspace_are_safe() {
        let mut app = WriterApp::new();
        let _ = app.set_client_bounds(0, 0);
        assert_eq!(app.content_rect().size(), sunlight_ui::Size::new(0, 0));
        let items = app.document_items();
        let canvas = app.document_canvas(items.as_slice());
        assert_eq!(canvas.viewport_size(), sunlight_ui::Size::new(0, 0));

        let _ = app.set_client_bounds(3, 1);
        assert_eq!(app.content_rect().size(), sunlight_ui::Size::new(3, 0));
    }

    #[test]
    fn grow_shrink_grow_layout_is_deterministic_and_identical_is_ignored() {
        let mut app = WriterApp::new();
        assert!(app.set_client_bounds(1500, 1000));
        let large = app.layout;
        assert!(!app.set_client_bounds(1500, 1000));
        assert!(app.set_client_bounds(320, 180));
        assert!(app.set_client_bounds(1500, 1000));
        assert_eq!(app.layout, large);
    }

    #[test]
    fn writer_starts_empty_and_routes_multiline_unicode_editing() {
        let mut app = WriterApp::new();
        assert_eq!(app.editor.text(), "");
        app.editor_focused = true;
        assert!(app.update(Event::key('م')));
        assert!(app.update(Event::key('\r')));
        assert!(app.update(Event::key('🐇')));
        assert_eq!(app.editor.text(), "م\n🐇");
        assert!(app.editor.text().is_char_boundary(app.editor.caret_byte()));
    }

    #[test]
    fn writer_keyboard_selection_replaces_across_lines() {
        let mut app = WriterApp::new();
        app.editor_focused = true;
        app.editor.insert_str("one\ntwo");
        let _ = app.configure_editor_layout();
        app.editor.set_caret(0, false);
        assert!(app.update(Event::key_press(
            super::KEY_DOWN,
            true,
            true,
            false,
            false,
            false,
        )));
        assert!(app.editor.selection_range().is_some());
        assert!(app.update(Event::key('X')));
        assert_eq!(app.editor.text(), "Xtwo");
    }

    #[test]
    fn writer_resize_reflows_without_changing_logical_text_or_caret() {
        let mut app = WriterApp::new();
        app.editor
            .insert_str("a long line that wraps repeatedly across a narrow writer page");
        let caret = app.editor.caret_byte();
        let text = String::from(app.editor.text());
        assert!(app.set_client_bounds(360, 260));
        let _ = app.configure_editor_layout();
        let narrow_lines = app.editor.lines().len();
        assert!(app.set_client_bounds(1400, 900));
        let _ = app.configure_editor_layout();
        assert_eq!(app.editor.text(), text);
        assert_eq!(app.editor.caret_byte(), caret);
        assert!(app.editor.lines().len() <= narrow_lines);
    }
}
