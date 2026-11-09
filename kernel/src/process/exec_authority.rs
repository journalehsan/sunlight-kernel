//! Image provenance is not a transferable process credential.
//! Call only after image preparation succeeds, under lifecycle serialization.
use crate::process::Process;

impl Process {
    /// Authenticated applications cannot acquire service provenance by naming
    /// an embedded daemon, including when the authenticated UID is root.
    pub(crate) fn restrict_account_member_roles(&mut self) {
        self.withdraw_exec_image_roles();
        self.system_allocation = false;
        self.service_identity = 0;
        self.trusted_display_service = false;
        self.trusted_swap_admin_service = false;
        self.trusted_zram_diagnostic = false;
        self.trusted_pty_service = false;
        self.trusted_kv_service = false;
        self.trusted_account_broker = false;
        self.trusted_wiseowl_braind = false;
    }

    pub(crate) fn withdraw_exec_image_roles(&mut self) {
        #[cfg(all(debug_assertions, feature = "fastlane_contract_test"))]
        {
            self.contract_service = None;
        }
        // These roles were installed by a trusted launch chain or an embedded
        // image. A replacement must not inherit permission to authenticate
        // sessions, establish login trust, or manage services merely by PID.
        self.trusted_auth_broker = false;
        self.trusted_vfs_service = false;
        self.trusted_service_manager = false;
        self.trusted_tty_session_service = false;
        self.trusted_lock_service = false;
        self.trusted_session_service = false;
    }
}
