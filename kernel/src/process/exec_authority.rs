//! Image provenance is not a transferable process credential.
//! Call only after image preparation succeeds, under lifecycle serialization.
use crate::process::Process;

impl Process {
    pub(crate) fn withdraw_exec_image_roles(&mut self) {
        // These roles were installed by a trusted launch chain or an embedded
        // image. A replacement must not inherit permission to authenticate
        // sessions, establish login trust, or manage services merely by PID.
        self.trusted_auth_broker = false;
        self.trusted_vfs_service = false;
        self.trusted_service_manager = false;
        self.trusted_tty_session_service = false;
        self.trusted_lock_service = false;
    }
}
