//! Startup completion uses all four transported words for the bundle ID.
//! No pointer, SHM allocation, or untransported tail words are needed.
use crate::{IpcMsg, SessionMsg, IPC_REGISTER_WORDS};

pub const MAX_APP_ID_BYTES: usize = IPC_REGISTER_WORDS * 8;

pub fn request(app_id: &str) -> Option<IpcMsg> {
    if app_id.is_empty() || app_id.len() > MAX_APP_ID_BYTES || app_id.as_bytes().contains(&0) {
        return None;
    }
    let mut msg = IpcMsg::with_label(SessionMsg::SESSION_STARTUP_COMPLETE_V2);
    for (i, byte) in app_id.bytes().enumerate() {
        msg.words[i / 8] |= (byte as u64) << ((i % 8) * 8);
    }
    msg.word_count = IPC_REGISTER_WORDS as u32;
    Some(msg)
}

pub fn decode(msg: &IpcMsg, buffer: &mut [u8; MAX_APP_ID_BYTES]) -> Option<usize> {
    if msg.label != SessionMsg::SESSION_STARTUP_COMPLETE_V2
        || msg.word_count as usize != IPC_REGISTER_WORDS
        || msg.cap_count != 0
    {
        return None;
    }
    for (i, byte) in buffer.iter_mut().enumerate() {
        *byte = (msg.words[i / 8] >> ((i % 8) * 8)) as u8;
    }
    let len = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(buffer.len());
    if len == 0 || buffer[len..].iter().any(|byte| *byte != 0) {
        return None;
    }
    core::str::from_utf8(&buffer[..len]).ok()?;
    Some(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_survives_register_transport_without_truncation() {
        for id in ["org.sunlight.welcome", "12345678901234567890123456789012"] {
            let sent = request(id).unwrap();
            let mut received = IpcMsg::with_label(sent.label);
            received.words[..IPC_REGISTER_WORDS].copy_from_slice(&sent.words[..IPC_REGISTER_WORDS]);
            received.word_count = sent.word_count;
            let mut bytes = [0; MAX_APP_ID_BYTES];
            let len = decode(&received, &mut bytes).unwrap();
            assert_eq!(&bytes[..len], id.as_bytes());
        }
    }

    #[test]
    fn invalid_or_incomplete_ids_are_rejected() {
        assert!(request("").is_none());
        assert!(request("123456789012345678901234567890123").is_none());
        assert!(request("org\0sunlight").is_none());
        let mut msg = request("org.sunlight.welcome").unwrap();
        let mut bytes = [0; MAX_APP_ID_BYTES];
        msg.word_count = 2;
        assert!(decode(&msg, &mut bytes).is_none());
        msg.word_count = 4;
        msg.words[0] &= !0xff;
        assert!(decode(&msg, &mut bytes).is_none());
    }
}
