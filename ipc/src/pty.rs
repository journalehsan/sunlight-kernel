use crate::{shm_alloc, shm_free, CapabilityToken, IpcMsg, PtyMsg, ShmError};

pub struct PtyBuffer {
    pointer: *mut u8,
    token: CapabilityToken,
}

impl PtyBuffer {
    pub fn new() -> Result<Self, ShmError> {
        let (pointer, token) = shm_alloc()?;
        Ok(Self { pointer, token })
    }

    pub fn request(
        &self,
        label: u64,
        id: u64,
        generation: u64,
        authority: CapabilityToken,
        length: usize,
    ) -> IpcMsg {
        IpcMsg::with_label(label)
            .word(0, id)
            .word(1, generation)
            .word(2, length.min(PtyMsg::BULK_BYTES) as u64)
            .with_cap(0, authority)
            .with_cap(1, self.token)
    }

    pub fn copy_from(&mut self, bytes: &[u8]) -> usize {
        let length = bytes.len().min(PtyMsg::BULK_BYTES);
        for (index, byte) in bytes[..length].iter().enumerate() {
            unsafe { self.pointer.add(index).write_volatile(*byte) };
        }
        length
    }

    pub fn copy_to(&self, bytes: &mut [u8], length: usize) -> usize {
        let length = length.min(bytes.len()).min(PtyMsg::BULK_BYTES);
        for (index, byte) in bytes[..length].iter_mut().enumerate() {
            *byte = unsafe { self.pointer.add(index).read_volatile() };
        }
        length
    }
}

impl Drop for PtyBuffer {
    fn drop(&mut self) {
        let _ = shm_free(self.token);
    }
}
