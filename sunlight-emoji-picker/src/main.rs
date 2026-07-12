#![no_std]
#![no_main]

use sun_font::{draw_text_centered, FontRole, TextStyle, VecFont};
use sunlight_ipc::{
    debug_log, ipc_call, nameserver_lookup_timeout, process_yield, shm_alloc, shm_free,
    CapabilityToken, ClipMsg, IpcMsg, ProcessExit, SHM_PAGE,
};
use sunlight_libc as libc;
use sunlight_ui::{
    request_close,
    widgets::TextInput,
    App, Canvas, Event, Point, Rect, Theme, Window, WindowConfig, WindowDecoration,
};

// ── Layout constants ───────────────────────────────────────────────────────────

const WIN_W: u32 = 340;
const WIN_H: u32 = 400;

const PAD: i32 = 8;
const SEARCH_H: u32 = 32;
const GRID_TOP: i32 = 56;
const CELL_SIZE: u32 = 46;
const CELL_GAP: u32 = 4;
const COLS: usize = 6;
const ROWS_VISIBLE: usize = 6;
const CELL_RADIUS: u32 = 8;

const GRID_X: i32 = (WIN_W as i32
    - (COLS as i32 * (CELL_SIZE + CELL_GAP) as i32 - CELL_GAP as i32))
    / 2;

// ── Clipboard wire format constants ────────────────────────────────────────────

const CLIP_MIME_TEXT: &[u8] = b"text/plain";
const CLIP_SOURCE_APP: &[u8] = b"sunlight-emoji-picker";
const CLIP_WIRE_MAGIC_SET: u32 = 0x4353_4554;
const CLIP_WIRE_VERSION: u16 = 1;

// ── Font statics ───────────────────────────────────────────────────────────────

static F_UI: VecFont = VecFont(FontRole::UiMedium);

// ── Keycodes ───────────────────────────────────────────────────────────────────

const KEY_ESC: u8 = 0x01;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;

// ── Emoji data ─────────────────────────────────────────────────────────────────

struct EmojiEntry {
    emoji: &'static str,
    shortcode: &'static str,
}

const ALL_EMOJIS: &[EmojiEntry] = &[
    EmojiEntry { emoji: "😀", shortcode: "grinning" },
    EmojiEntry { emoji: "😃", shortcode: "big eyes" },
    EmojiEntry { emoji: "😄", shortcode: "grin" },
    EmojiEntry { emoji: "😁", shortcode: "beaming" },
    EmojiEntry { emoji: "😆", shortcode: "laughing" },
    EmojiEntry { emoji: "😂", shortcode: "tears of joy" },
    EmojiEntry { emoji: "🤣", shortcode: "rolling" },
    EmojiEntry { emoji: "😊", shortcode: "blushing" },
    EmojiEntry { emoji: "😇", shortcode: "angel" },
    EmojiEntry { emoji: "😍", shortcode: "heart eyes" },
    EmojiEntry { emoji: "😘", shortcode: "kiss" },
    EmojiEntry { emoji: "😜", shortcode: "winking tongue" },
    EmojiEntry { emoji: "😎", shortcode: "cool" },
    EmojiEntry { emoji: "🤩", shortcode: "star struck" },
    EmojiEntry { emoji: "🥳", shortcode: "party" },
    EmojiEntry { emoji: "😏", shortcode: "smirk" },
    EmojiEntry { emoji: "😒", shortcode: "unamused" },
    EmojiEntry { emoji: "😔", shortcode: "pensive" },
    EmojiEntry { emoji: "😴", shortcode: "sleepy" },
    EmojiEntry { emoji: "🤒", shortcode: "sick" },
    EmojiEntry { emoji: "🤕", shortcode: "injured" },
    EmojiEntry { emoji: "🥵", shortcode: "hot" },
    EmojiEntry { emoji: "🥶", shortcode: "cold" },
    EmojiEntry { emoji: "😱", shortcode: "screaming" },
    EmojiEntry { emoji: "😢", shortcode: "crying" },
    EmojiEntry { emoji: "😡", shortcode: "angry" },
    EmojiEntry { emoji: "🤬", shortcode: "cursing" },
    EmojiEntry { emoji: "💀", shortcode: "skull" },
    EmojiEntry { emoji: "👻", shortcode: "ghost" },
    EmojiEntry { emoji: "👽", shortcode: "alien" },
    EmojiEntry { emoji: "🤖", shortcode: "robot" },
    EmojiEntry { emoji: "🎃", shortcode: "pumpkin" },
    EmojiEntry { emoji: "🤡", shortcode: "clown" },
    EmojiEntry { emoji: "💩", shortcode: "poop" },
    EmojiEntry { emoji: "👍", shortcode: "thumbs up" },
    EmojiEntry { emoji: "👎", shortcode: "thumbs down" },
    EmojiEntry { emoji: "👏", shortcode: "clap" },
    EmojiEntry { emoji: "🙌", shortcode: "raised hands" },
    EmojiEntry { emoji: "🤝", shortcode: "handshake" },
    EmojiEntry { emoji: "✊", shortcode: "fist" },
    EmojiEntry { emoji: "🤞", shortcode: "crossed fingers" },
    EmojiEntry { emoji: "✌️", shortcode: "peace" },
    EmojiEntry { emoji: "🤘", shortcode: "rock on" },
    EmojiEntry { emoji: "👌", shortcode: "ok hand" },
    EmojiEntry { emoji: "👉", shortcode: "point right" },
    EmojiEntry { emoji: "👈", shortcode: "point left" },
    EmojiEntry { emoji: "👇", shortcode: "point down" },
    EmojiEntry { emoji: "👆", shortcode: "point up" },
    EmojiEntry { emoji: "🤙", shortcode: "call me" },
    EmojiEntry { emoji: "💪", shortcode: "muscle" },
    EmojiEntry { emoji: "🖖", shortcode: "vulcan" },
    EmojiEntry { emoji: "✋", shortcode: "raised hand" },
    EmojiEntry { emoji: "🙏", shortcode: "pray" },
    EmojiEntry { emoji: "🐱", shortcode: "cat" },
    EmojiEntry { emoji: "🐶", shortcode: "dog" },
    EmojiEntry { emoji: "🐰", shortcode: "rabbit" },
    EmojiEntry { emoji: "🦊", shortcode: "fox" },
    EmojiEntry { emoji: "🐻", shortcode: "bear" },
    EmojiEntry { emoji: "🐼", shortcode: "panda" },
    EmojiEntry { emoji: "🐯", shortcode: "tiger" },
    EmojiEntry { emoji: "🦁", shortcode: "lion" },
    EmojiEntry { emoji: "🐮", shortcode: "cow" },
    EmojiEntry { emoji: "🐷", shortcode: "pig" },
    EmojiEntry { emoji: "🐸", shortcode: "frog" },
    EmojiEntry { emoji: "🐵", shortcode: "monkey" },
    EmojiEntry { emoji: "🐔", shortcode: "chicken" },
    EmojiEntry { emoji: "🐧", shortcode: "penguin" },
    EmojiEntry { emoji: "🦅", shortcode: "eagle" },
    EmojiEntry { emoji: "🦉", shortcode: "owl" },
    EmojiEntry { emoji: "🐝", shortcode: "bee" },
    EmojiEntry { emoji: "🦋", shortcode: "butterfly" },
    EmojiEntry { emoji: "🐢", shortcode: "turtle" },
    EmojiEntry { emoji: "🐍", shortcode: "snake" },
    EmojiEntry { emoji: "🦎", shortcode: "lizard" },
    EmojiEntry { emoji: "🦖", shortcode: "t-rex" },
    EmojiEntry { emoji: "🐙", shortcode: "octopus" },
    EmojiEntry { emoji: "🦀", shortcode: "crab" },
    EmojiEntry { emoji: "🐠", shortcode: "fish" },
    EmojiEntry { emoji: "🐬", shortcode: "dolphin" },
    EmojiEntry { emoji: "🐳", shortcode: "whale" },
    EmojiEntry { emoji: "🦈", shortcode: "shark" },
    EmojiEntry { emoji: "🐊", shortcode: "crocodile" },
    EmojiEntry { emoji: "🐆", shortcode: "leopard" },
    EmojiEntry { emoji: "🦓", shortcode: "zebra" },
    EmojiEntry { emoji: "🐘", shortcode: "elephant" },
    EmojiEntry { emoji: "🦒", shortcode: "giraffe" },
    EmojiEntry { emoji: "🐎", shortcode: "horse" },
    EmojiEntry { emoji: "🦄", shortcode: "unicorn" },
    EmojiEntry { emoji: "🐑", shortcode: "sheep" },
    EmojiEntry { emoji: "🐐", shortcode: "goat" },
    EmojiEntry { emoji: "🦜", shortcode: "parrot" },
    EmojiEntry { emoji: "🐇", shortcode: "bunny" },
    EmojiEntry { emoji: "🦥", shortcode: "sloth" },
    EmojiEntry { emoji: "🐾", shortcode: "paws" },
    EmojiEntry { emoji: "🐉", shortcode: "dragon" },
    EmojiEntry { emoji: "🌵", shortcode: "cactus" },
    EmojiEntry { emoji: "🌲", shortcode: "evergreen" },
    EmojiEntry { emoji: "🌴", shortcode: "palm tree" },
    EmojiEntry { emoji: "🍀", shortcode: "four leaf clover" },
    EmojiEntry { emoji: "🌻", shortcode: "sunflower" },
    EmojiEntry { emoji: "🌹", shortcode: "rose" },
    EmojiEntry { emoji: "🌸", shortcode: "cherry blossom" },
    EmojiEntry { emoji: "🌺", shortcode: "hibiscus" },
    EmojiEntry { emoji: "🍄", shortcode: "mushroom" },
    EmojiEntry { emoji: "🌎", shortcode: "earth" },
    EmojiEntry { emoji: "🌙", shortcode: "moon" },
    EmojiEntry { emoji: "⭐", shortcode: "star" },
    EmojiEntry { emoji: "✨", shortcode: "sparkles" },
    EmojiEntry { emoji: "⚡", shortcode: "lightning" },
    EmojiEntry { emoji: "🔥", shortcode: "fire" },
    EmojiEntry { emoji: "🌈", shortcode: "rainbow" },
    EmojiEntry { emoji: "☀️", shortcode: "sun" },
    EmojiEntry { emoji: "☁️", shortcode: "cloud" },
    EmojiEntry { emoji: "❄️", shortcode: "snowflake" },
    EmojiEntry { emoji: "⛄", shortcode: "snowman" },
    EmojiEntry { emoji: "💧", shortcode: "droplet" },
    EmojiEntry { emoji: "🌊", shortcode: "wave" },
    EmojiEntry { emoji: "🍎", shortcode: "apple" },
    EmojiEntry { emoji: "🍌", shortcode: "banana" },
    EmojiEntry { emoji: "🍇", shortcode: "grape" },
    EmojiEntry { emoji: "🍓", shortcode: "strawberry" },
    EmojiEntry { emoji: "🍒", shortcode: "cherry" },
    EmojiEntry { emoji: "🍑", shortcode: "peach" },
    EmojiEntry { emoji: "🍋", shortcode: "lemon" },
    EmojiEntry { emoji: "🍉", shortcode: "watermelon" },
    EmojiEntry { emoji: "🥑", shortcode: "avocado" },
    EmojiEntry { emoji: "🧀", shortcode: "cheese" },
    EmojiEntry { emoji: "🍕", shortcode: "pizza" },
    EmojiEntry { emoji: "🍔", shortcode: "hamburger" },
    EmojiEntry { emoji: "🌮", shortcode: "taco" },
    EmojiEntry { emoji: "🌯", shortcode: "burrito" },
    EmojiEntry { emoji: "🍣", shortcode: "sushi" },
    EmojiEntry { emoji: "🍜", shortcode: "ramen" },
    EmojiEntry { emoji: "🎂", shortcode: "cake" },
    EmojiEntry { emoji: "🍩", shortcode: "doughnut" },
    EmojiEntry { emoji: "🍪", shortcode: "cookie" },
    EmojiEntry { emoji: "🍦", shortcode: "ice cream" },
    EmojiEntry { emoji: "🍺", shortcode: "beer" },
    EmojiEntry { emoji: "🍷", shortcode: "wine" },
    EmojiEntry { emoji: "🍸", shortcode: "cocktail" },
    EmojiEntry { emoji: "☕", shortcode: "coffee" },
    EmojiEntry { emoji: "🍵", shortcode: "tea" },
    EmojiEntry { emoji: "⚽", shortcode: "soccer" },
    EmojiEntry { emoji: "🏀", shortcode: "basketball" },
    EmojiEntry { emoji: "🏈", shortcode: "football" },
    EmojiEntry { emoji: "⚾", shortcode: "baseball" },
    EmojiEntry { emoji: "🎾", shortcode: "tennis" },
    EmojiEntry { emoji: "🎱", shortcode: "pool" },
    EmojiEntry { emoji: "⛳", shortcode: "golf" },
    EmojiEntry { emoji: "🎣", shortcode: "fishing" },
    EmojiEntry { emoji: "🥊", shortcode: "boxing" },
    EmojiEntry { emoji: "🎮", shortcode: "game" },
    EmojiEntry { emoji: "🎲", shortcode: "dice" },
    EmojiEntry { emoji: "🎵", shortcode: "music" },
    EmojiEntry { emoji: "🎤", shortcode: "microphone" },
    EmojiEntry { emoji: "🎸", shortcode: "guitar" },
    EmojiEntry { emoji: "🎹", shortcode: "piano" },
    EmojiEntry { emoji: "🎺", shortcode: "trumpet" },
    EmojiEntry { emoji: "🥁", shortcode: "drum" },
    EmojiEntry { emoji: "🎨", shortcode: "paint" },
    EmojiEntry { emoji: "🧩", shortcode: "puzzle" },
    EmojiEntry { emoji: "🎯", shortcode: "bullseye" },
    EmojiEntry { emoji: "🚗", shortcode: "car" },
    EmojiEntry { emoji: "🚌", shortcode: "bus" },
    EmojiEntry { emoji: "🚲", shortcode: "bicycle" },
    EmojiEntry { emoji: "✈️", shortcode: "airplane" },
    EmojiEntry { emoji: "🚀", shortcode: "rocket" },
    EmojiEntry { emoji: "🛸", shortcode: "ufo" },
    EmojiEntry { emoji: "🚁", shortcode: "helicopter" },
    EmojiEntry { emoji: "⛵", shortcode: "sailboat" },
    EmojiEntry { emoji: "🚢", shortcode: "ship" },
    EmojiEntry { emoji: "🚂", shortcode: "train" },
    EmojiEntry { emoji: "🏠", shortcode: "house" },
    EmojiEntry { emoji: "🏢", shortcode: "office" },
    EmojiEntry { emoji: "🏥", shortcode: "hospital" },
    EmojiEntry { emoji: "🏫", shortcode: "school" },
    EmojiEntry { emoji: "🏰", shortcode: "castle" },
    EmojiEntry { emoji: "🗼", shortcode: "tokyo tower" },
    EmojiEntry { emoji: "🗽", shortcode: "liberty" },
    EmojiEntry { emoji: "⛪", shortcode: "church" },
    EmojiEntry { emoji: "🕌", shortcode: "mosque" },
    EmojiEntry { emoji: "🏖️", shortcode: "beach" },
    EmojiEntry { emoji: "🌋", shortcode: "volcano" },
    EmojiEntry { emoji: "📱", shortcode: "phone" },
    EmojiEntry { emoji: "💻", shortcode: "laptop" },
    EmojiEntry { emoji: "🖥️", shortcode: "desktop" },
    EmojiEntry { emoji: "⌨️", shortcode: "keyboard" },
    EmojiEntry { emoji: "🖱️", shortcode: "mouse" },
    EmojiEntry { emoji: "💾", shortcode: "floppy" },
    EmojiEntry { emoji: "💿", shortcode: "cd" },
    EmojiEntry { emoji: "📷", shortcode: "camera" },
    EmojiEntry { emoji: "🎥", shortcode: "film" },
    EmojiEntry { emoji: "📺", shortcode: "tv" },
    EmojiEntry { emoji: "📻", shortcode: "radio" },
    EmojiEntry { emoji: "⏰", shortcode: "alarm" },
    EmojiEntry { emoji: "⌛", shortcode: "hourglass" },
    EmojiEntry { emoji: "🔋", shortcode: "battery" },
    EmojiEntry { emoji: "💡", shortcode: "light bulb" },
    EmojiEntry { emoji: "🔦", shortcode: "flashlight" },
    EmojiEntry { emoji: "💸", shortcode: "money" },
    EmojiEntry { emoji: "💰", shortcode: "money bag" },
    EmojiEntry { emoji: "💳", shortcode: "credit card" },
    EmojiEntry { emoji: "💎", shortcode: "gem" },
    EmojiEntry { emoji: "🔧", shortcode: "wrench" },
    EmojiEntry { emoji: "🔨", shortcode: "hammer" },
    EmojiEntry { emoji: "🧲", shortcode: "magnet" },
    EmojiEntry { emoji: "🔫", shortcode: "water pistol" },
    EmojiEntry { emoji: "💣", shortcode: "bomb" },
    EmojiEntry { emoji: "🔪", shortcode: "kitchen knife" },
    EmojiEntry { emoji: "🗡️", shortcode: "dagger" },
    EmojiEntry { emoji: "🛡️", shortcode: "shield" },
    EmojiEntry { emoji: "🔮", shortcode: "crystal ball" },
    EmojiEntry { emoji: "🔭", shortcode: "telescope" },
    EmojiEntry { emoji: "🔬", shortcode: "microscope" },
    EmojiEntry { emoji: "💊", shortcode: "pill" },
    EmojiEntry { emoji: "💉", shortcode: "syringe" },
    EmojiEntry { emoji: "🧪", shortcode: "test tube" },
    EmojiEntry { emoji: "🔑", shortcode: "key" },
    EmojiEntry { emoji: "🔒", shortcode: "locked" },
    EmojiEntry { emoji: "🔓", shortcode: "unlocked" },
    EmojiEntry { emoji: "🛒", shortcode: "shopping cart" },
    EmojiEntry { emoji: "🎁", shortcode: "gift" },
    EmojiEntry { emoji: "🎈", shortcode: "balloon" },
    EmojiEntry { emoji: "🎉", shortcode: "party popper" },
    EmojiEntry { emoji: "📚", shortcode: "books" },
    EmojiEntry { emoji: "📌", shortcode: "pushpin" },
    EmojiEntry { emoji: "✂️", shortcode: "scissors" },
    EmojiEntry { emoji: "✏️", shortcode: "pencil" },
    EmojiEntry { emoji: "❤️", shortcode: "red heart" },
    EmojiEntry { emoji: "🧡", shortcode: "orange heart" },
    EmojiEntry { emoji: "💛", shortcode: "yellow heart" },
    EmojiEntry { emoji: "💚", shortcode: "green heart" },
    EmojiEntry { emoji: "💙", shortcode: "blue heart" },
    EmojiEntry { emoji: "💜", shortcode: "purple heart" },
    EmojiEntry { emoji: "🖤", shortcode: "black heart" },
    EmojiEntry { emoji: "🤍", shortcode: "white heart" },
    EmojiEntry { emoji: "💔", shortcode: "broken heart" },
    EmojiEntry { emoji: "💯", shortcode: "hundred" },
    EmojiEntry { emoji: "💥", shortcode: "boom" },
    EmojiEntry { emoji: "💬", shortcode: "speech" },
    EmojiEntry { emoji: "💤", shortcode: "sleep" },
    EmojiEntry { emoji: "⚠️", shortcode: "warning" },
    EmojiEntry { emoji: "🚫", shortcode: "prohibited" },
    EmojiEntry { emoji: "✅", shortcode: "check" },
    EmojiEntry { emoji: "❌", shortcode: "cross" },
    EmojiEntry { emoji: "❓", shortcode: "question" },
    EmojiEntry { emoji: "❗", shortcode: "exclamation" },
    EmojiEntry { emoji: "♻️", shortcode: "recycle" },
    EmojiEntry { emoji: "🔴", shortcode: "red circle" },
    EmojiEntry { emoji: "🟠", shortcode: "orange circle" },
    EmojiEntry { emoji: "🟡", shortcode: "yellow circle" },
    EmojiEntry { emoji: "🟢", shortcode: "green circle" },
    EmojiEntry { emoji: "🔵", shortcode: "blue circle" },
    EmojiEntry { emoji: "🟣", shortcode: "purple circle" },
    EmojiEntry { emoji: "⚫", shortcode: "black circle" },
    EmojiEntry { emoji: "⚪", shortcode: "white circle" },
    EmojiEntry { emoji: "⬛", shortcode: "black square" },
    EmojiEntry { emoji: "⬜", shortcode: "white square" },
    EmojiEntry { emoji: "▶️", shortcode: "play" },
    EmojiEntry { emoji: "⏸️", shortcode: "pause" },
    EmojiEntry { emoji: "⏹️", shortcode: "stop" },
    EmojiEntry { emoji: "➡️", shortcode: "right arrow" },
    EmojiEntry { emoji: "⬅️", shortcode: "left arrow" },
    EmojiEntry { emoji: "⬆️", shortcode: "up arrow" },
    EmojiEntry { emoji: "⬇️", shortcode: "down arrow" },
    EmojiEntry { emoji: "🔄", shortcode: "refresh" },
    EmojiEntry { emoji: "📎", shortcode: "paperclip" },
    EmojiEntry { emoji: "🏁", shortcode: "checkered flag" },
    EmojiEntry { emoji: "🚩", shortcode: "red flag" },
];

// ── Alloc stub ─────────────────────────────────────────────────────────────────

struct NoAlloc;
unsafe impl core::alloc::GlobalAlloc for NoAlloc {
    unsafe fn alloc(&self, _layout: core::alloc::Layout) -> *mut u8 {
        core::ptr::null_mut()
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}
#[global_allocator]
static ALLOC: NoAlloc = NoAlloc;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[EMOJI] panic\n");
    loop {
        process_yield();
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn ascii_contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let hb = haystack.as_bytes();
    let nb = needle.as_bytes();
    if nb.len() > hb.len() {
        return false;
    }
    for start in 0..=hb.len() - nb.len() {
        if hb[start..start + nb.len()]
            .iter()
            .zip(nb.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            return true;
        }
    }
    false
}

fn ensure_clipboard_service() -> Option<CapabilityToken> {
    if let Some(cap) = nameserver_lookup_timeout("clipd", 50) {
        return Some(cap);
    }
    let _ = libc::spawn(b"/sbin/sunlight-clipd", &[b"sunlight-clipd"], None)
        .or_else(|_| libc::spawn(b"/bin/sunlight-clipd", &[b"sunlight-clipd"], None));
    for _ in 0..8 {
        if let Some(cap) = nameserver_lookup_timeout("clipd", 75) {
            return Some(cap);
        }
        process_yield();
    }
    None
}

fn set_clipboard_text(text: &str) -> Result<(), &'static str> {
    let cap = ensure_clipboard_service().ok_or("Clipboard service unavailable")?;
    let payload = text.as_bytes();
    let total_len = 16 + CLIP_MIME_TEXT.len() + CLIP_SOURCE_APP.len() + payload.len();
    if total_len > SHM_PAGE {
        return Err("Clipboard payload too large");
    }
    let (ptr, token) = shm_alloc().map_err(|_| "shm_alloc failed")?;
    unsafe {
        let buf = core::slice::from_raw_parts_mut(ptr, SHM_PAGE);
        let mut pos = 0usize;
        buf[pos..pos + 4].copy_from_slice(&CLIP_WIRE_MAGIC_SET.to_le_bytes());
        pos += 4;
        buf[pos..pos + 2].copy_from_slice(&CLIP_WIRE_VERSION.to_le_bytes());
        pos += 2;
        buf[pos] = 1;
        pos += 1;
        buf[pos] = 1;
        pos += 1;
        let mime_len = CLIP_MIME_TEXT.len() as u16;
        buf[pos..pos + 2].copy_from_slice(&mime_len.to_le_bytes());
        pos += 2;
        let source_len = CLIP_SOURCE_APP.len() as u16;
        buf[pos..pos + 2].copy_from_slice(&source_len.to_le_bytes());
        pos += 2;
        let payload_len = payload.len() as u32;
        buf[pos..pos + 4].copy_from_slice(&payload_len.to_le_bytes());
        pos += 4;
        buf[pos..pos + CLIP_MIME_TEXT.len()].copy_from_slice(CLIP_MIME_TEXT);
        pos += CLIP_MIME_TEXT.len();
        buf[pos..pos + CLIP_SOURCE_APP.len()].copy_from_slice(CLIP_SOURCE_APP);
        pos += CLIP_SOURCE_APP.len();
        buf[pos..pos + payload.len()].copy_from_slice(payload);
    }
    let reply = ipc_call(
        cap,
        IpcMsg::with_label(ClipMsg::SET_CLIPBOARD)
            .word(0, total_len as u64)
            .with_cap(0, token),
    );
    let _ = shm_free(token);
    if reply.label == ClipMsg::ERROR {
        return Err("Clipboard error");
    }
    Ok(())
}

// ── Application state ──────────────────────────────────────────────────────────

struct EmojiPickerApp {
    search: TextInput<'static, 64>,
    hovered_idx: Option<usize>,
    scroll_offset: usize,
    filtered_indices: [usize; 256],
    filtered_count: usize,
}

impl EmojiPickerApp {
    fn new() -> Self {
        let mut app = Self {
            search: TextInput::new(Rect::new(
                PAD,
                10,
                WIN_W - (PAD as u32) * 2,
                SEARCH_H,
            ))
            .with_font(&F_UI)
            .with_placeholder("Search emoji..."),
            hovered_idx: None,
            scroll_offset: 0,
            filtered_indices: [0; 256],
            filtered_count: 0,
        };
        app.filter_emojis();
        app
    }

    fn filter_emojis(&mut self) {
        self.filtered_count = 0;
        let query = self.search.value();
        for (i, entry) in ALL_EMOJIS.iter().enumerate() {
            if self.filtered_count >= self.filtered_indices.len() {
                break;
            }
            if ascii_contains_ignore_case(entry.shortcode, query) {
                self.filtered_indices[self.filtered_count] = i;
                self.filtered_count += 1;
            }
        }
        self.scroll_offset = 0;
        self.hovered_idx = None;
    }

    fn cell_rect(idx: usize) -> Rect {
        let row = idx / COLS;
        let col = idx % COLS;
        Rect::new(
            GRID_X + col as i32 * (CELL_SIZE + CELL_GAP) as i32,
            GRID_TOP + row as i32 * (CELL_SIZE + CELL_GAP) as i32,
            CELL_SIZE,
            CELL_SIZE,
        )
    }

    fn hit_test(&self, x: i32, y: i32) -> Option<usize> {
        let visible = ROWS_VISIBLE.min(self.filtered_count.saturating_sub(self.scroll_offset));
        for i in 0..visible {
            let rect = Self::cell_rect(i);
            if rect.contains(Point::new(x, y)) {
                return Some(i + self.scroll_offset);
            }
        }
        None
    }

    fn draw_emoji_cell(
        canvas: &mut Canvas,
        idx: usize,
        entry: &EmojiEntry,
        hovered: bool,
        theme: &Theme,
    ) {
        let rect = Self::cell_rect(idx);
        let bg = if hovered {
            theme.accent.darken(170)
        } else {
            theme.panel
        };
        let border = if hovered {
            theme.accent
        } else {
            theme.border.lighten(24)
        };
        canvas.fill_rounded_rect_with_border(rect, CELL_RADIUS, bg, border, 1);
        draw_text_centered(
            canvas,
            rect,
            entry.emoji,
            &TextStyle::new(FontRole::Emoji, theme.text),
        );
    }

    fn copy_emoji(&self, global_idx: usize) -> bool {
        if let Some(&emoji_idx) = self.filtered_indices.get(global_idx) {
            if let Some(entry) = ALL_EMOJIS.get(emoji_idx) {
                let _ = set_clipboard_text(entry.emoji);
                return true;
            }
        }
        false
    }
}

// ── App trait ──────────────────────────────────────────────────────────────────

impl App for EmojiPickerApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);

        self.search.draw(canvas, theme);

        let visible = ROWS_VISIBLE.min(self.filtered_count.saturating_sub(self.scroll_offset));
        for i in 0..visible {
            let global_idx = i + self.scroll_offset;
            if let Some(&emoji_idx) = self.filtered_indices.get(global_idx) {
                if let Some(entry) = ALL_EMOJIS.get(emoji_idx) {
                    Self::draw_emoji_cell(
                        canvas,
                        i,
                        entry,
                        self.hovered_idx == Some(global_idx),
                        theme,
                    );
                }
            }
        }
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Click { x, y } => {
                let was_active = self.search.active;
                let search_clicked = self.search.update(Event::Click { x, y });

                if search_clicked {
                    return true;
                }

                if was_active && !self.search.active {
                    return true;
                }

                if let Some(global_idx) = self.hit_test(x, y) {
                    self.copy_emoji(global_idx);
                    request_close();
                    return true;
                }

                false
            }
            Event::MouseMove { x, y } => {
                let new_hover = self.hit_test(x, y);
                if new_hover != self.hovered_idx {
                    self.hovered_idx = new_hover;
                    return true;
                }
                false
            }
            Event::Key(ch) => {
                if self.search.active {
                    let old_len = self.search.value().len();
                    let changed = self.search.update(Event::Key(ch));
                    if changed && self.search.value().len() != old_len {
                        self.filter_emojis();
                    }
                    return changed;
                }
                false
            }
            Event::KeyPress {
                keycode,
                pressed: true,
                ..
            } => {
                if self.search.active {
                    if keycode == KEY_ESC {
                        self.search.active = false;
                        return true;
                    }
                    let old_len = self.search.value().len();
                    let changed = self.search.update(Event::KeyPress {
                        keycode,
                        pressed: true,
                        shift: false,
                        ctrl: false,
                        alt: false,
                        super_key: false,
                    });
                    if changed && self.search.value().len() != old_len {
                        self.filter_emojis();
                    }
                    return changed;
                }

                match keycode {
                    KEY_ESC => {
                        request_close();
                        return true;
                    }
                    KEY_UP => {
                        if self.scroll_offset > 0 {
                            self.scroll_offset -= 1;
                            return true;
                        }
                    }
                    KEY_DOWN => {
                        if self.scroll_offset + ROWS_VISIBLE < self.filtered_count {
                            self.scroll_offset += 1;
                            return true;
                        }
                    }
                    _ => {}
                }
                false
            }
            _ => false,
        }
    }
}

// ── Entry point ────────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut app = EmojiPickerApp::new();

    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Sunlight Emoji Picker",
        decoration: WindowDecoration::HiddenOverlay,
    }) {
        Some(w) => w,
        None => {
            debug_log("[EMOJI] failed to connect window\n");
            loop {
                process_yield();
            }
        }
    };

    window.run(&mut app);
    ProcessExit::exit(0);
}
