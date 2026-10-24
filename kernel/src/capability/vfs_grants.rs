//! Narrow production UAC -> requester -> VFS authority. Caller holds SCHEDULER
//! and CAP_BROKER through identity validation, proof consumption and insertion.
use super::{CapError, CapabilityBroker, CapabilityToken, GrantDelivery, RecipientIdentity};

impl CapabilityBroker {
    pub(super) fn delegate_vfs_grants(
        &mut self,
        proof: CapabilityToken,
        delivery: GrantDelivery,
        path: Option<&str>,
        now: u64,
    ) -> Result<heapless::Vec<CapabilityToken, 2>, CapError> {
        if path.is_some_and(|p| !sunlight_ipc::vfs_grants::executable_path(p)) {
            return Err(CapError::InsufficientRights);
        }
        // A cache entry, UID in a message, or nonzero token is not a root
        // password proof. Never let UAC consume a different requester's proof.
        let auth = self
            .auth_session_grants
            .iter()
            .position(|g| {
                g.token == proof
                    && g.uid == 0
                    && now <= g.expires_at_tick
                    && g.delivery.is_some_and(|d| d.usable(delivery.recipient))
            })
            .ok_or(CapError::InvalidToken)?;
        let expires = self.auth_session_grants[auth].expires_at_tick;
        self.vfs_caps
            .retain(|g| g.delivery.is_none() || now <= g.expires);
        let original = self.vfs_caps.len();
        let mut tokens = heapless::Vec::new();
        let exact = [path.unwrap_or("")];
        let prefixes: &[&str] = if path.is_some() {
            &exact
        } else {
            &super::ELEVATED_EXEC_PREFIXES
        };
        for prefix in prefixes {
            let mut allowed_prefix = super::String::new();
            allowed_prefix
                .push_str(prefix)
                .map_err(|_| CapError::InsufficientRights)?;
            let cap = super::VfsCapability {
                allowed_prefix,
                flags: super::AccessFlags::READ_EXECUTE,
            };
            let token = match self.grant_vfs_capability(delivery.recipient.pid, cap) {
                Ok(token) => token,
                Err(error) => {
                    // Roll back only this unpublished batch; the proof stays
                    // valid for retry. Consumed random candidates are discarded.
                    self.vfs_caps.truncate(original);
                    return Err(error);
                }
            };
            let grant = self.vfs_caps.last_mut().expect("just inserted grant");
            grant.delivery = Some(delivery);
            grant.expires = expires;
            // Two fixed base prefixes or one exact path; capacity is proven.
            let _ = tokens.push(token);
        }
        self.auth_session_grants.swap_remove(auth);
        Ok(tokens)
    }

    pub(super) fn consume_vfs_grant(
        &mut self,
        token: CapabilityToken,
        recipient: RecipientIdentity,
        path: &str,
        now: u64,
    ) -> bool {
        if !sunlight_ipc::vfs_grants::executable_path(path) {
            return false;
        }
        let Some(index) = self.vfs_caps.iter().position(|g| {
            g.token == token && now <= g.expires && g.delivery.is_some_and(|d| d.usable(recipient))
        }) else {
            return false;
        };
        let prefix = self.vfs_caps[index].cap.allowed_prefix.as_str();
        // Only the two fixed base policies are directory grants. An exact
        // command grant must not authorize descendants or sibling prefixes.
        let allowed = if super::ELEVATED_EXEC_PREFIXES.contains(&prefix) {
            path.strip_prefix(prefix)
                .is_some_and(|tail| tail.starts_with('/'))
        } else {
            path == prefix
        };
        if allowed {
            self.vfs_caps.swap_remove(index);
        }
        allowed
    }

    pub(super) fn cancel_vfs_grant(
        &mut self,
        token: CapabilityToken,
        recipient: RecipientIdentity,
    ) {
        self.vfs_caps
            .retain(|g| g.token != token || !g.delivery.is_some_and(|d| d.recipient == recipient));
    }
}
