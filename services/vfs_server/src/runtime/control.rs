//! Bounded service loop shared by the native copied channel and host adapters.
use super::Runtime;
use sunlight_ipc::vfs_rpc::{
    registration::Registration,
    retention::{transport, wire::FRAME_BYTES},
};

/// Implementations must obtain identity/lifecycle evidence independently of
/// request payloads. Ordinary IPC must never implement this trusted interface.
pub trait Control {
    fn derive_source(
        &mut self,
        _epoch: u64,
        _bytes: &[u8; FRAME_BYTES],
    ) -> Option<sunlight_ipc::vfs_rpc::retention::Receipt> {
        None
    }
    fn chdir(
        &mut self,
        _epoch: u64,
        _bytes: &[u8; FRAME_BYTES],
    ) -> Option<[u8; sunlight_ipc::vfs_rpc::chdir_transport::CONTEXT_BYTES]> {
        None
    }
    fn finish_chdir(
        &mut self,
        _epoch: u64,
        _reply: &[u8; FRAME_BYTES],
        _errno: u64,
        _cwd: &str,
    ) -> bool {
        false
    }
    /// Pull the oldest copied file request for one independently authenticated
    /// registration. Repeating the call returns the same request until its
    /// exact reply is accepted, which lets the service revalidate publication.
    fn file_next(
        &mut self,
        _epoch: u64,
    ) -> Result<Option<[u8; sunlight_ipc::vfs_rpc::file_transport::FRAME_BYTES]>, ()> {
        Err(())
    }
    fn file_reply(
        &mut self,
        _epoch: u64,
        _bytes: &[u8; sunlight_ipc::vfs_rpc::file_transport::FRAME_BYTES],
    ) -> bool {
        false
    }
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
    fn derive_source(
        &mut self,
        epoch: u64,
        bytes: &[u8; FRAME_BYTES],
    ) -> Option<sunlight_ipc::vfs_rpc::retention::Receipt> {
        transport::derive_source(epoch, bytes)
    }
    fn chdir(
        &mut self,
        epoch: u64,
        bytes: &[u8; FRAME_BYTES],
    ) -> Option<[u8; sunlight_ipc::vfs_rpc::chdir_transport::CONTEXT_BYTES]> {
        sunlight_ipc::vfs_rpc::chdir_transport::context(epoch, bytes)
    }
    fn finish_chdir(
        &mut self,
        epoch: u64,
        reply: &[u8; FRAME_BYTES],
        errno: u64,
        cwd: &str,
    ) -> bool {
        sunlight_ipc::vfs_rpc::chdir_transport::finish(epoch, reply, errno, cwd)
    }
    fn file_next(
        &mut self,
        epoch: u64,
    ) -> Result<Option<[u8; sunlight_ipc::vfs_rpc::file_transport::FRAME_BYTES]>, ()> {
        sunlight_ipc::vfs_rpc::file_transport::next(epoch)
    }
    fn file_reply(
        &mut self,
        epoch: u64,
        bytes: &[u8; sunlight_ipc::vfs_rpc::file_transport::FRAME_BYTES],
    ) -> bool {
        sunlight_ipc::vfs_rpc::file_transport::reply(epoch, bytes)
    }

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
                if let Some(context) = control.chdir(registration.epoch, &bytes) {
                    if !self.chdir_control(control, registration, &bytes, &context) {
                        break;
                    }
                    continue;
                }
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
            for _ in 0..8 {
                let Ok(Some(bytes)) = control.file_next(registration.epoch) else {
                    break;
                };
                if !self.dispatch_file(control, registration, &bytes) {
                    break;
                }
            }
        }
    }

    fn chdir_control(
        &mut self,
        control: &mut impl Control,
        authenticated: Registration,
        bytes: &[u8; FRAME_BYTES],
        context: &[u8; sunlight_ipc::vfs_rpc::chdir_transport::CONTEXT_BYTES],
    ) -> bool {
        use super::wire::Frame;
        use crate::policy::Error;
        use sunlight_fs::FsError;
        let Ok(Frame::Retain {
            registration,
            scope,
            ..
        }) = Frame::decode(bytes)
        else {
            return false;
        };
        if registration != authenticated {
            return false;
        }
        let Ok(c) = sunlight_ipc::vfs_rpc::chdir_transport::decode_context(context) else {
            return false;
        };
        // Revalidation is independent of the retain payload and happens before
        // publication. The finish syscall rechecks lifecycle at kernel commit.
        let mut saved = self
            .policy_mut(authenticated)
            .and_then(|p| p.save_directory(c.old))
            .ok();
        let change = match c.kind {
            sunlight_ipc::vfs_rpc::chdir_transport::Kind::Directory => Self::change_directory,
            sunlight_ipc::vfs_rpc::chdir_transport::Kind::Root => Self::change_root,
        };
        let changed = change(
            self,
            authenticated,
            c.old,
            scope,
            c.path,
            sunlight_fs::Credential {
                uid: c.uid,
                gid: c.gid,
            },
            || control.chdir(authenticated.epoch, bytes).as_ref() == Some(context),
        );
        let errno_for = |error: &Error| match error {
            Error::Filesystem(FsError::NotFound) => 2,
            Error::Filesystem(FsError::NotDir) => 20,
            Error::Filesystem(FsError::PermissionDenied | FsError::OperationNotPermitted) => 13,
            Error::Filesystem(FsError::InvalidPath) => 22,
            Error::Filesystem(FsError::NoMemory) => 12,
            Error::Filesystem(FsError::Unsupported) => 95,
            _ => 5,
        };
        let mut errno = changed.as_ref().err().map_or(0, errno_for);
        let prepared = if changed.is_ok() {
            self.policy_mut(authenticated)
                .and_then(|p| p.cwd(scope))
                .and_then(|cwd| self.request(authenticated, bytes).map(|reply| (reply, cwd)))
        } else {
            self.policy_mut(authenticated)
                .and_then(|p| p.refuse(authenticated, bytes))
                .map(|reply| (reply, alloc::string::String::from("/")))
        };
        let (reply, cwd) = match prepared {
            Ok((reply, cwd)) => {
                if changed.is_ok() && !matches!(reply, Frame::Retained(_)) {
                    if let Some(saved) = saved.take() {
                        self.policy_mut(authenticated)
                            .unwrap()
                            .restore_directory(scope, saved);
                    }
                    errno = 12;
                }
                (reply, cwd)
            }
            Err(error) if changed.is_ok() => {
                // Failure obtaining a representable getcwd result must leave
                // the old policy intact. Refusal owns replay accounting.
                if let Some(saved) = saved.take() {
                    self.policy_mut(authenticated)
                        .unwrap()
                        .restore_directory(scope, saved);
                }
                errno = errno_for(&error);
                let Ok(reply) = self
                    .policy_mut(authenticated)
                    .and_then(|p| p.refuse(authenticated, bytes))
                else {
                    return false;
                };
                (reply, alloc::string::String::from("/"))
            }
            Err(_) => return false,
        };
        let Ok(encoded) = reply.encode() else {
            return false;
        };
        let retained = matches!(reply, Frame::Retained(_));
        let accepted = control.finish_chdir(authenticated.epoch, &encoded, errno, &cwd);
        if !accepted || !retained {
            let _ = self.withdraw(authenticated, scope);
            if let Frame::Retained(receipt) = reply {
                // A rejected commit cannot leak the provisional remote lease.
                let _ = self.request(authenticated, &Frame::Release(receipt).encode().unwrap());
            }
        }
        accepted
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
            if let Some(source) = control.derive_source(authenticated.epoch, bytes) {
                let installed =
                    self.policy_mut(authenticated)?
                        .derive_retained_caller(source, scope, || {
                            control.derive_source(authenticated.epoch, bytes) == Some(source)
                        });
                if installed.is_err() {
                    return self.policy_mut(authenticated)?.refuse(authenticated, bytes);
                }
                let result = self.request(authenticated, bytes);
                if !matches!(result, Ok(Frame::Retained(_))) {
                    self.withdraw(authenticated, scope)?;
                }
                return result;
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
        let frame = Frame::decode(bytes).map_err(crate::policy::Error::Protocol)?;
        if let Frame::CallerRetired {
            registration,
            caller,
            event_id,
        }
        | Frame::FileOwnerRetired {
            registration,
            caller,
            event_id,
        } = frame
        {
            if registration != authenticated {
                return Err(crate::policy::Error::Identity);
            }
            self.policy_mut(authenticated)?;
            self.retire_caller(caller);
            if matches!(frame, Frame::FileOwnerRetired { .. }) {
                self.retire_file_owner(caller);
                return Ok(Frame::FileOwnerRetiredAck {
                    registration,
                    caller,
                    event_id,
                });
            }
            return Ok(Frame::CallerRetiredAck {
                registration,
                caller,
                event_id,
            });
        }
        self.request(authenticated, bytes)
    }
}
