//! Exact register-IPC envelope for the experimental operation. No IPC is sent
//! here: the service must validate the kernel sender and committed connection.
use crate::{
    fastlane_operation::{Error, Frame},
    IpcMsg, IPC_REGISTER_WORDS,
};

const REQUEST: u64 = 0x6b01_0010;
const RESPONSE: u64 = 0x6b02_0010;
const _: () = assert!(IPC_REGISTER_WORDS == 4);

impl Frame {
    pub fn ipc_message(&self) -> IpcMsg {
        let mut message = IpcMsg::with_label(if self.is_response() {
            RESPONSE
        } else {
            REQUEST
        });
        for (index, word) in self.ipc_words().iter().enumerate() {
            message = message.word(index, *word);
        }
        message
    }

    pub fn from_ipc_message(message: &IpcMsg) -> Result<Self, Error> {
        if message.word_count as usize != IPC_REGISTER_WORDS || message.cap_count != 0 {
            return Err(Error::Format);
        }
        let frame = Self::from_ipc(&message.words[..IPC_REGISTER_WORDS], 0)?;
        if message.label
            != if frame.is_response() {
                RESPONSE
            } else {
                REQUEST
            }
        {
            return Err(Error::Format);
        }
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_ipc_envelope_and_response() {
        let request = Frame::request(9, b"0123456789abcdef").unwrap();
        for frame in [request, request.transform().unwrap()] {
            let msg = frame.ipc_message();
            assert_eq!(msg.word_count, 4);
            assert_eq!(msg.cap_count, 0);
            assert_eq!(Frame::from_ipc_message(&msg), Ok(frame));
        }
    }
    #[test]
    fn malformed_envelope_never_decodes() {
        let request = Frame::request(9, b"abc").unwrap();
        let original = request.ipc_message();
        for word_count in [0, 1, 2, 3, 5, 8, u32::MAX] {
            let mut msg = original;
            msg.word_count = word_count;
            assert_eq!(Frame::from_ipc_message(&msg), Err(Error::Format));
        }
        for cap_count in [1, 2, u32::MAX] {
            let mut msg = original;
            msg.cap_count = cap_count;
            assert_eq!(Frame::from_ipc_message(&msg), Err(Error::Format));
        }
        let mut msg = original;
        msg.label = RESPONSE;
        assert_eq!(Frame::from_ipc_message(&msg), Err(Error::Format));
        msg.label = 0;
        assert_eq!(Frame::from_ipc_message(&msg), Err(Error::Format));
    }
}
