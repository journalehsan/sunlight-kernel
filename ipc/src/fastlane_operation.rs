//! Bounded fixture wire contract. This module supplies no identity or authority.
//! Native users must authenticate both participants and authorize the operation
//! before committing; a decoded frame never authorizes its sender.
pub const OPERATION: u16 = 0x10;
pub const VERSION: u8 = 1;
pub const MAX_PAYLOAD: usize = 16;
pub const FRAME_BYTES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Format,
    Duplicate,
    Terminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    request_id: u64,
    response: bool,
    len: u8,
    payload: [u8; MAX_PAYLOAD],
}

impl Frame {
    pub fn request_id(&self) -> u64 {
        self.request_id
    }

    pub fn is_response(&self) -> bool {
        self.response
    }

    pub fn request(request_id: u64, bytes: &[u8]) -> Result<Self, Error> {
        if request_id == 0 || bytes.len() > MAX_PAYLOAD {
            return Err(Error::Format);
        }
        let mut payload = [0; MAX_PAYLOAD];
        payload[..bytes.len()].copy_from_slice(bytes);
        Ok(Self {
            request_id,
            response: false,
            len: bytes.len() as u8,
            payload,
        })
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload[..self.len as usize]
    }

    pub fn transform(&self) -> Result<Self, Error> {
        if self.response {
            return Err(Error::Format);
        }
        let mut reply = *self;
        reply.response = true;
        reply.payload[..self.len as usize].reverse();
        Ok(reply)
    }

    /// Exactly four register words, excluding the fixed IPC message label.
    pub fn ipc_words(&self) -> [u64; 4] {
        [
            OPERATION as u64
                | (VERSION as u64) << 16
                | (u64::from(self.response)) << 24
                | (self.len as u64) << 32,
            self.request_id,
            u64::from_le_bytes(self.payload[..8].try_into().unwrap()),
            u64::from_le_bytes(self.payload[8..].try_into().unwrap()),
        ]
    }

    pub fn from_ipc(words: &[u64], cap_count: usize) -> Result<Self, Error> {
        if words.len() != 4 || cap_count != 0 {
            return Err(Error::Format);
        }
        let header = words[0];
        let len = ((header >> 32) & 0xff) as usize;
        if header as u16 != OPERATION
            || (header >> 16) as u8 != VERSION
            || ((header >> 24) & 0xff) > 1
            || header >> 40 != 0
            || len > MAX_PAYLOAD
            || words[1] == 0
        {
            return Err(Error::Format);
        }
        let mut payload = [0; MAX_PAYLOAD];
        payload[..8].copy_from_slice(&words[2].to_le_bytes());
        payload[8..].copy_from_slice(&words[3].to_le_bytes());
        if payload[len..].iter().any(|b| *b != 0) {
            return Err(Error::Format);
        }
        Ok(Self {
            request_id: words[1],
            response: (header >> 24) & 1 != 0,
            len: len as u8,
            payload,
        })
    }

    /// Independent byte framing for a private snapshot from the hostile ring.
    pub fn ring_bytes(&self) -> [u8; FRAME_BYTES] {
        let mut bytes = [0; FRAME_BYTES];
        bytes[..2].copy_from_slice(&OPERATION.to_le_bytes());
        bytes[2] = VERSION;
        bytes[3] = u8::from(self.response);
        bytes[4] = self.len;
        bytes[8..16].copy_from_slice(&self.request_id.to_le_bytes());
        bytes[16..].copy_from_slice(&self.payload);
        bytes
    }

    pub fn from_ring(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != FRAME_BYTES {
            return Err(Error::Format);
        }
        let len = bytes[4] as usize;
        if u16::from_le_bytes([bytes[0], bytes[1]]) != OPERATION
            || bytes[2] != VERSION
            || bytes[3] > 1
            || len > MAX_PAYLOAD
            || bytes[5..8] != [0; 3]
            || bytes[16 + len..].iter().any(|b| *b != 0)
        {
            return Err(Error::Format);
        }
        let request_id = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        if request_id == 0 {
            return Err(Error::Format);
        }
        let mut payload = [0; MAX_PAYLOAD];
        payload.copy_from_slice(&bytes[16..]);
        Ok(Self {
            request_id,
            response: bytes[3] != 0,
            len: len as u8,
            payload,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Ipc,
    Ring,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    Failed,
}

/// Application bookkeeping only. The kernel contract owns connection authority,
/// transport selection and commit. Construct this only after its successful
/// commit; it cannot be used as a replacement for that check.
pub struct Operation {
    transport: Transport,
    request_id: Option<u64>,
    outcome: Option<Outcome>,
}
impl Operation {
    pub fn committed(transport: Transport) -> Self {
        Self {
            transport,
            request_id: None,
            outcome: None,
        }
    }
    pub fn transport(&self) -> Transport {
        self.transport
    }
    pub fn outcome(&self) -> Option<Outcome> {
        self.outcome
    }
    pub fn execute(&mut self, request: Frame) -> Result<Frame, Error> {
        if self.request_id == Some(request.request_id) {
            return Err(Error::Duplicate);
        }
        if self.outcome.is_some() {
            return Err(Error::Terminal);
        }
        self.request_id = Some(request.request_id);
        match request.transform() {
            Ok(reply) => {
                self.outcome = Some(Outcome::Completed);
                Ok(reply)
            }
            Err(error) => {
                self.outcome = Some(Outcome::Failed);
                Err(error)
            }
        }
    }
    /// A decode failure or peer closure ends this operation without retry.
    pub fn fail(&mut self) -> Result<(), Error> {
        if self.outcome.is_some() {
            return Err(Error::Terminal);
        }
        self.outcome = Some(Outcome::Failed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn operation_equivalence_includes_outcomes_and_frozen_transport() {
        for len in 0..=MAX_PAYLOAD {
            let bytes: [u8; MAX_PAYLOAD] = core::array::from_fn(|i| (i * 13 + 7) as u8);
            let request = Frame::request(7, &bytes[..len]).unwrap();
            let mut ipc = Operation::committed(Transport::Ipc);
            let mut ring = Operation::committed(Transport::Ring);
            let a = ipc
                .execute(Frame::from_ipc(&request.ipc_words(), 0).unwrap())
                .unwrap();
            let b = ring
                .execute(Frame::from_ring(&request.ring_bytes()).unwrap())
                .unwrap();
            assert_eq!(a, b);
            assert_eq!(a.request_id, 7);
            assert!(a.response);
            let mut expected = bytes;
            expected[..len].reverse();
            assert_eq!(a.payload(), &expected[..len]);
            assert_eq!(Frame::from_ipc(&a.ipc_words(), 0), Ok(a));
            assert_eq!(Frame::from_ring(&b.ring_bytes()), Ok(b));
            for op in [&mut ipc, &mut ring] {
                assert_eq!(op.outcome(), Some(Outcome::Completed));
                assert_eq!(op.execute(request), Err(Error::Duplicate));
                assert_eq!(
                    op.execute(Frame::request(8, b"late").unwrap()),
                    Err(Error::Terminal)
                );
                assert_eq!(op.fail(), Err(Error::Terminal));
                assert_eq!(op.outcome(), Some(Outcome::Completed));
            }
            assert_eq!(ipc.transport(), Transport::Ipc);
            assert_eq!(ring.transport(), Transport::Ring);
        }
    }
    #[test]
    fn independent_wire_encodings_have_fixed_golden_layout() {
        let request = Frame::request(0x0807060504030201, b"abc").unwrap();
        let words = request.ipc_words();
        assert_eq!(words, [0x0000000300010010, 0x0807060504030201, 0x636261, 0]);
        let bytes = request.ring_bytes();
        assert_eq!(&bytes[..8], &[16, 0, 1, 0, 3, 0, 0, 0]);
        for i in 0..4 {
            assert_eq!(&bytes[i * 8..i * 8 + 8], words[i].to_le_bytes());
        }
    }
    #[test]
    fn failure_is_terminal_and_responses_cannot_be_requests() {
        let request = Frame::request(1, b"a").unwrap();
        for transport in [Transport::Ipc, Transport::Ring] {
            let mut op = Operation::committed(transport);
            assert_eq!(op.execute(request.transform().unwrap()), Err(Error::Format));
            assert_eq!(op.outcome(), Some(Outcome::Failed));
            assert_eq!(op.execute(request), Err(Error::Duplicate));
            assert_eq!(op.fail(), Err(Error::Terminal));
            let mut op = Operation::committed(transport);
            assert_eq!(op.fail(), Ok(()));
            assert_eq!(op.execute(request), Err(Error::Terminal));
        }
    }
    #[test]
    fn parser_negative_controls() {
        assert_eq!(Frame::request(0, b""), Err(Error::Format));
        assert_eq!(Frame::request(1, &[0; 17]), Err(Error::Format));
        let request = Frame::request(1, b"abc").unwrap();
        let words = request.ipc_words();
        let bytes = request.ring_bytes();
        for n in 0..4 {
            assert_eq!(Frame::from_ipc(&words[..n], 0), Err(Error::Format));
        }
        assert_eq!(Frame::from_ipc(&[0; 5], 0), Err(Error::Format));
        assert_eq!(Frame::from_ipc(&words, 1), Err(Error::Format));
        for n in 0..32 {
            assert_eq!(Frame::from_ring(&bytes[..n]), Err(Error::Format));
        }
        assert_eq!(Frame::from_ring(&[0; 33]), Err(Error::Format));
        // Unknown opcode/version/kind, excessive length, reserved bits, ID zero,
        // noncanonical padding. Mutations are checked through both decoders.
        for (offset, value) in [
            (0, 17),
            (2, 2),
            (3, 2),
            (4, 17),
            (5, 1),
            (6, 1),
            (7, 1),
            (8, 0),
            (19, 1),
            (31, 1),
        ] {
            let mut bad = bytes;
            bad[offset] = value;
            let bad_words: [u64; 4] = core::array::from_fn(|i| {
                u64::from_le_bytes(bad[i * 8..i * 8 + 8].try_into().unwrap())
            });
            assert_eq!(Frame::from_ring(&bad), Err(Error::Format));
            assert_eq!(Frame::from_ipc(&bad_words, 0), Err(Error::Format));
        }
    }
}
