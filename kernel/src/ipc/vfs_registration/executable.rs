//! Eager loader input from one authenticated executable description.
use super::{file, AdmissionIdentity, Receipt};
#[cfg(not(test))]
use super::{Scheduler, RUNTIME};
use alloc::{string::String, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use sunlight_ipc::vfs_rpc::file_transport as api;

pub(crate) type Authority = (AdmissionIdentity, Receipt);

static BUFFER_LIVE: AtomicBool = AtomicBool::new(false);

pub(crate) struct BufferSlot;

impl BufferSlot {
    fn acquire() -> Result<Self, file::Error> {
        BUFFER_LIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| file::Error::Capacity)?;
        Ok(Self)
    }
}

impl Drop for BufferSlot {
    fn drop(&mut self) {
        BUFFER_LIVE.store(false, Ordering::Release);
    }
}

pub(crate) fn route<'a>(
    path: &'a str,
    cwd: &str,
    service_cwd: bool,
) -> Option<alloc::borrow::Cow<'a, str>> {
    use alloc::borrow::Cow;
    use sunlight_fs::path::service_downloads;

    if path.starts_with('/') {
        return service_downloads(path).then_some(Cow::Borrowed(path));
    }
    if service_cwd {
        return Some(Cow::Borrowed(path));
    }
    let joined = alloc::format!("{}/{}", cwd.trim_end_matches('/'), path);
    service_downloads(&joined).then_some(Cow::Owned(joined))
}

#[cfg(not(test))]
pub(crate) fn admission(
    pid: usize,
    sched: &Scheduler,
    caps: &super::CapabilityBroker,
) -> Option<Authority> {
    let identity = super::capture(pid, sched, caps).ok()?;
    let receipt = RUNTIME.lock().caller_receipt(identity)?;
    Some((identity, receipt))
}

#[cfg(not(test))]
pub(crate) fn settle(caller: super::Caller) {
    RUNTIME.lock().settle_executable(caller);
}

pub(crate) struct Source {
    pub(crate) path: String,
    pub(crate) authority: (AdmissionIdentity, Receipt),
    token: u64,
    bytes: Vec<u8>,
    size: usize,
    pub(crate) buffer_slot: Option<BufferSlot>,
    #[cfg(test)]
    pub(super) fail_reserve: bool,
}

impl Source {
    pub(crate) fn new(path: String, authority: (AdmissionIdentity, Receipt)) -> Self {
        Self {
            path,
            authority,
            token: 0,
            bytes: Vec::new(),
            size: 0,
            buffer_slot: None,
            #[cfg(test)]
            fail_reserve: false,
        }
    }

    pub(crate) fn step(
        &mut self,
        mut call: impl FnMut(
            file::FileOperation<'_>,
            Option<(super::Caller, u64)>,
        ) -> Result<file::FileCall, file::Error>,
    ) -> Result<Option<Vec<u8>>, file::Error> {
        let identity = self.authority.0;
        let epoch = identity.registration.epoch;
        if self.token == 0 {
            let file::FileCall::Complete(reply) = call(
                file::FileOperation::Open {
                    anchor: 0,
                    path: &self.path,
                    access: api::ACCESS_READ | api::OPEN_EXECUTABLE,
                    mode: 0,
                },
                None,
            )?
            else {
                return Ok(None);
            };
            if reply.errno != 0
                || reply.registration_epoch != epoch
                || reply.result0 == 0
                || reply.result1 > api::MAX_EXECUTABLE_BYTES as u64
            {
                return Err(file::Error::Protocol);
            }
            self.token = reply.result0;
            self.size = reply.result1 as usize;
            let slot = BufferSlot::acquire()?;
            #[cfg(test)]
            if self.fail_reserve {
                return Err(file::Error::Capacity);
            }
            self.bytes
                .try_reserve_exact(self.size)
                .map_err(|_| file::Error::Capacity)?;
            self.buffer_slot = Some(slot);
        }
        while self.bytes.len() < self.size {
            let length = (self.size - self.bytes.len()).min(api::IO_BYTES);
            let file::FileCall::Complete(reply) = call(
                file::FileOperation::Pread {
                    token: self.token,
                    offset: self.bytes.len() as u64,
                    length,
                },
                Some((identity.caller, epoch)),
            )?
            else {
                return Ok(None);
            };
            if reply.errno != 0
                || reply.registration_epoch != epoch
                || reply.result0 != length as u64
                || reply.payload.len() != length
            {
                return Err(file::Error::Protocol);
            }
            self.bytes.extend_from_slice(&reply.payload);
        }
        Ok(Some(core::mem::take(&mut self.bytes)))
    }
}
