//! Bounded service loop shared by the native copied channel and host adapters.
use super::Runtime;
use sunlight_ipc::vfs_rpc::{
    registration::Registration,
    retention::{transport, wire::FRAME_BYTES},
};

/// Implementations must obtain identity/lifecycle evidence independently of
/// request payloads. Ordinary IPC must never implement this trusted interface.
pub trait Control {
    fn snapshot(&mut self) -> Option<Registration>;
    fn status(&mut self, epoch: u64) -> Result<(Registration, bool), ()>;
    fn next(&mut self, epoch: u64) -> Result<Option<[u8; FRAME_BYTES]>, ()>;
    fn reply(&mut self, epoch: u64, bytes: &[u8; FRAME_BYTES]) -> bool;
    /// Independent kernel policy check, never inferred from `next` payloads.
    fn authorize_root(&mut self, _epoch: u64, _bytes: &[u8; FRAME_BYTES]) -> bool {
        false
    }
    fn initial_cwd(
        &mut self,
        epoch: u64,
        bytes: &[u8; FRAME_BYTES],
    ) -> Option<[u8; transport::CWD_BYTES]> {
        self.authorize_root(epoch, bytes)
            .then(|| transport::encode_cwd("/").unwrap())
    }
}

pub struct CopiedControl;
impl Control for CopiedControl {
    fn initial_cwd(
        &mut self,
        epoch: u64,
        bytes: &[u8; FRAME_BYTES],
    ) -> Option<[u8; transport::CWD_BYTES]> {
        transport::initial_cwd(epoch, bytes)
    }
    fn authorize_root(&mut self, epoch: u64, bytes: &[u8; FRAME_BYTES]) -> bool {
        transport::authorize_root(epoch, bytes)
    }
    fn snapshot(&mut self) -> Option<Registration> {
        transport::snapshot()
    }
    fn status(&mut self, epoch: u64) -> Result<(Registration, bool), ()> {
        transport::status(epoch)
    }
    fn next(&mut self, epoch: u64) -> Result<Option<[u8; FRAME_BYTES]>, ()> {
        transport::next(epoch)
    }
    fn reply(&mut self, epoch: u64, bytes: &[u8; FRAME_BYTES]) -> bool {
        transport::reply(epoch, bytes)
    }
}

impl<const P: usize, const L: usize, const R: usize> Runtime<P, L, R> {
    /// Poll at ordinary service turns and kernel control wakeups.
    /// Every retained epoch gets 16 attempts, preventing a retrying old release
    /// from starving a replacement. Missing status proves no retirement/drain.
    pub fn poll_control(&mut self, control: &mut impl Control) {
        self.poll_control_inner(control, None);
    }

    /// Production initial-cwd policy handoff uses the same namespace as legacy
    /// handlers. Existing component policies use `poll_control` without grants.
    pub fn poll_root_control(
        &mut self,
        control: &mut impl Control,
        namespace: &crate::directory::DirectoryNamespace,
    ) {
        self.poll_control_inner(control, Some(namespace));
    }

    fn poll_control_inner(
        &mut self,
        control: &mut impl Control,
        namespace: Option<&crate::directory::DirectoryNamespace>,
    ) {
        if let Some(registration) = control.snapshot() {
            let _ = self.observe_active(registration);
        }
        for registration in self.registrations().into_iter().flatten() {
            let Ok((authenticated, retired)) = control.status(registration.epoch) else {
                continue;
            };
            if authenticated != registration {
                continue;
            }
            if retired && self.retire_registration(registration).is_err() {
                continue;
            }
            for _ in 0..16 {
                let Ok(Some(bytes)) = control.next(registration.epoch) else {
                    break;
                };
                let result = if let Some(namespace) = namespace {
                    self.root_control_request(control, registration, namespace, &bytes)
                } else {
                    self.control_request(registration, &bytes)
                };
                let Ok(reply) = result else {
                    break;
                };
                let Ok(bytes) = reply.encode() else {
                    break;
                };
                if !control.reply(registration.epoch, &bytes) {
                    break;
                }
            }
        }
    }

    fn root_control_request(
        &mut self,
        control: &mut impl Control,
        authenticated: Registration,
        namespace: &crate::directory::DirectoryNamespace,
        bytes: &[u8; FRAME_BYTES],
    ) -> Result<super::wire::Frame, crate::policy::Error> {
        use super::wire::Frame;
        use crate::policy::Error;
        if let Frame::Retain {
            registration,
            scope,
            ..
        } = Frame::decode(bytes).map_err(Error::Protocol)?
        {
            if registration != authenticated {
                return Err(Error::Identity);
            }
            let Some(profile) = control.initial_cwd(authenticated.epoch, bytes) else {
                return self.policy_mut(authenticated)?.refuse(authenticated, bytes);
            };
            let directory = transport::decode_cwd(&profile).map_err(Error::Protocol)?;
            // Exclusive runtime access prevents retain handling between pinning
            // and revalidation. The kernel checks lifecycle again on the reply;
            // a race after this check owns compensation rather than a binding.
            let installed = self.install(authenticated, scope, namespace, "/", directory);
            if installed.is_ok() && control.initial_cwd(authenticated.epoch, bytes) != Some(profile)
            {
                self.withdraw(authenticated, scope)?;
                return self.policy_mut(authenticated)?.refuse(authenticated, bytes);
            }
            // Refuse through replay accounting without consulting an older
            // caller binding, including on capacity/pin/allocation failure.
            if installed.is_err() {
                return self.policy_mut(authenticated)?.refuse(authenticated, bytes);
            }
            let result = self.request(authenticated, bytes);
            if !matches!(result, Ok(Frame::Retained(_))) {
                // Lease-table exhaustion/refusal cannot leave provisional policy
                // pins behind after the kernel settles the denied preparation.
                self.withdraw(authenticated, scope)?;
            }
            return result;
        }
        self.control_request(authenticated, bytes)
    }

    /// Only the independently authenticated copied control channel may call
    /// this entrypoint. Ordinary retention routing cannot retire caller policy.
    pub fn control_request(
        &mut self,
        authenticated: Registration,
        bytes: &[u8],
    ) -> Result<super::wire::Frame, crate::policy::Error> {
        use super::wire::Frame;
        if let Frame::CallerRetired {
            registration,
            caller,
            event_id,
        } = Frame::decode(bytes).map_err(crate::policy::Error::Protocol)?
        {
            if registration != authenticated {
                return Err(crate::policy::Error::Identity);
            }
            self.policy_mut(authenticated)?;
            self.retire_caller(caller);
            return Ok(Frame::CallerRetiredAck {
                registration,
                caller,
                event_id,
            });
        }
        self.request(authenticated, bytes)
    }
}
