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
    // Query
    Status  = 10,
    List    = 11,
    // Logging
    GetLog  = 20,
}

impl SunlightdOp {
    pub fn from_u32(val: u32) -> Option<Self> {
        match val {
            1 => Some(Self::Start),
            2 => Some(Self::Stop),
            3 => Some(Self::Restart),
            4 => Some(Self::Reload),
            10 => Some(Self::Status),
            11 => Some(Self::List),
            20 => Some(Self::GetLog),
            _ => None,
        }
    }
}

/// Extract unit name from IPC message (packed in words[0..2])
pub fn extract_unit_name(msg: &IpcMsg) -> heapless::String<64> {
    let mut name = heapless::String::new();
    
    // First 32 bytes from words[0..4]
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

/// Pack unit name into IPC message words
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

/// Status reply structure (packed into IpcMsg)
#[derive(Debug, Clone, Copy)]
pub struct StatusReply {
    pub state: u32,      // ServiceState discriminant
    pub pid: u32,
    pub restarts: u32,
    pub started_at: u64,
}

impl StatusReply {
    pub fn pack(&self, msg: &mut IpcMsg) {
        msg.words[0] = self.state as u64;
        msg.words[1] = self.pid as u64;
        msg.words[2] = self.restarts as u64;
        msg.words[3] = self.started_at;
    }
}

/// List entry structure
#[derive(Debug, Clone)]
pub struct ListEntry {
    pub name: heapless::String<64>,
    pub state: u32,
    pub pid: u32,
    pub restarts: u32,
}

impl ListEntry {
    pub fn pack(&self, msg: &mut IpcMsg) {
        pack_unit_name(msg, &self.name);
        msg.words[4] = self.state as u64;
        msg.words[5] = self.pid as u64;
        msg.words[6] = self.restarts as u64;
    }
}
