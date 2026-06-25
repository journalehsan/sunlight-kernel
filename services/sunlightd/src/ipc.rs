//! IPC control interface for sunlightd
//! Defines the control opcodes and message handling

use sunlight_ipc::IpcMsg;

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SunlightdOp {
    // Management
    Start   = 1,
    Stop    = 2,
    Restart = 3,
    Reload  = 4,
    Enable  = 5,
    Disable = 6,
    // Query
    Status  = 10,
    List    = 11,
    // Logging
    GetLog  = 20,
}

impl SunlightdOp {
    pub fn from_u32(val: u32) -> Option<Self> {
        match val {
            1  => Some(Self::Start),
            2  => Some(Self::Stop),
            3  => Some(Self::Restart),
            4  => Some(Self::Reload),
            5  => Some(Self::Enable),
            6  => Some(Self::Disable),
            10 => Some(Self::Status),
            11 => Some(Self::List),
            20 => Some(Self::GetLog),
            _  => None,
        }
    }
}

/// Extract unit name from IPC message words[0..4] (up to 32 bytes).
pub fn extract_unit_name(msg: &IpcMsg) -> heapless::String<64> {
    let mut name = heapless::String::new();
    for i in 0..4 {
        let word = msg.words[i];
        for j in 0..8 {
            let byte = ((word >> (j * 8)) & 0xff) as u8;
            if byte == 0 {
                return name;
            }
            let _ = name.push(byte as char);
        }
    }
    name
}

/// Pack unit name into IPC message words[0..4] (up to 32 bytes).
pub fn pack_unit_name(msg: &mut IpcMsg, name: &str) {
    let bytes = name.as_bytes();
    for i in 0..4 {
        let mut word: u64 = 0;
        for j in 0..8 {
            let idx = i * 8 + j;
            if idx < bytes.len() {
                word |= (bytes[idx] as u64) << (j * 8);
            }
        }
        msg.words[i] = word;
    }
}

/// Status reply (packed into words[0..4]).
///
/// words[0] = state
/// words[1] = pid
/// words[2] = restarts (low 32) | enabled (bit 32)
/// words[3] = started_at
#[derive(Debug, Clone, Copy)]
pub struct StatusReply {
    pub state: u32,
    pub pid: u32,
    pub restarts: u32,
    pub started_at: u64,
    pub enabled: bool,
}

impl StatusReply {
    pub fn pack(&self, msg: &mut IpcMsg) {
        msg.words[0] = self.state as u64;
        msg.words[1] = self.pid as u64;
        msg.words[2] = self.restarts as u64 | ((self.enabled as u64) << 32);
        msg.words[3] = self.started_at;
    }
}

/// List entry packed into words[0..4] (transport-safe: IPC carries words[0..4] only).
///
/// words[0] = total(u32) | state(u8)<<32 | enabled(u1)<<40 | restarts(u8)<<48
/// words[1] = pid(u32)
/// words[2] = name bytes  0..8  little-endian
/// words[3] = name bytes  8..16 little-endian
#[derive(Debug, Clone)]
pub struct ListEntry {
    pub name: heapless::String<64>,
    pub state: u32,
    pub pid: u32,
    pub restarts: u32,
    pub enabled: bool,
}

impl ListEntry {
    pub fn pack(&self, msg: &mut IpcMsg, total: usize) {
        msg.words[0] = (total as u64) & 0xFFFF_FFFF
            | ((self.state as u64 & 0xFF) << 32)
            | ((self.enabled as u64) << 40)
            | ((self.restarts as u64 & 0xFF) << 48);
        msg.words[1] = self.pid as u64;

        let bytes = self.name.as_bytes();
        let mut w2: u64 = 0;
        let mut w3: u64 = 0;
        for i in 0..8.min(bytes.len()) {
            w2 |= (bytes[i] as u64) << (i * 8);
        }
        for i in 8..16.min(bytes.len()) {
            w3 |= (bytes[i] as u64) << ((i - 8) * 8);
        }
        msg.words[2] = w2;
        msg.words[3] = w3;
    }
}
