//! Production delegation request validation. Mint authorization is performed
//! by the kernel at commit, never by a nonzero-token or payload UID test.
use sunlight_ipc::{vfs_grants as api, CapabilityToken, IpcMsg};

pub fn handle(msg: &IpcMsg) -> IpcMsg {
    dispatch(msg, api::mint)
}

fn dispatch(
    msg: &IpcMsg,
    mint: impl FnOnce(CapabilityToken, Option<&str>) -> Option<IpcMsg>,
) -> IpcMsg {
    let denied = || IpcMsg::with_label(api::REPLY_ERR);
    if msg.cap_count != 1 || msg.caps[0] == CapabilityToken::INVALID || msg.words[0] != 0 {
        return denied();
    }
    match msg.label {
        api::BASE_CAPS if msg.word_count == 1 => mint(msg.caps[0], None).unwrap_or_else(denied),
        api::DELEGATE if (2..=4).contains(&msg.word_count) => {
            let mut bytes = [0u8; 24];
            for (word, chunk) in msg.words[1..4].iter().zip(bytes.chunks_exact_mut(8)) {
                chunk.copy_from_slice(&word.to_le_bytes());
            }
            let Some(end) = bytes.iter().position(|b| *b == 0) else {
                return denied();
            };
            if end >= (msg.word_count as usize - 1) * 8 {
                return denied();
            }
            let Ok(path) = core::str::from_utf8(&bytes[..end]) else {
                return denied();
            };
            if !api::executable_path(path) {
                return denied();
            }
            mint(msg.caps[0], Some(path)).unwrap_or_else(denied)
        }
        _ => denied(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nonzero_bearer_is_not_an_administrative_proof() {
        let msg = IpcMsg::with_label(api::BASE_CAPS)
            .word(0, 0)
            .with_cap(0, CapabilityToken(123));
        let reply = dispatch(&msg, |_, _| None);
        assert_eq!(reply.label, api::REPLY_ERR);
        assert_eq!(reply.cap_count, 0);
    }
    #[test]
    fn valid_base_request_forwards_only_kernel_grants() {
        let msg = IpcMsg::with_label(api::BASE_CAPS)
            .word(0, 0)
            .with_cap(0, CapabilityToken(123));
        let reply = dispatch(&msg, |proof, path| {
            assert_eq!(proof.0, 123);
            assert!(path.is_none());
            Some(
                IpcMsg::with_label(api::REPLY_OK)
                    .with_cap(0, CapabilityToken(456))
                    .with_cap(1, CapabilityToken(789)),
            )
        });
        assert_eq!(reply.cap_count, 2);
        assert_eq!(reply.caps[0].0, 456);
        assert_ne!(reply.caps[0], msg.caps[0]);
    }
    #[test]
    fn malformed_requests_do_not_reach_mint() {
        for fault in 0..5 {
            let mut msg = IpcMsg::with_label(api::BASE_CAPS)
                .word(0, 0)
                .with_cap(0, CapabilityToken(123));
            match fault {
                0 => msg.cap_count = 0,
                1 => msg.words[0] = 1000,
                2 => msg.word_count = 0,
                3 => msg.label = 99,
                _ => msg.cap_count = 2,
            }
            assert_eq!(
                dispatch(&msg, |_, _| panic!("invalid request reached mint")).label,
                api::REPLY_ERR
            );
        }
    }
    #[test]
    fn exact_path_and_traversal_are_distinguished() {
        for path in ["/bin/true", "/bin/../etc/x"] {
            let mut msg = IpcMsg::with_label(api::DELEGATE)
                .word(0, 0)
                .word(3, 0)
                .with_cap(0, CapabilityToken(123));
            for (i, b) in path.bytes().enumerate() {
                msg.words[1 + i / 8] |= (b as u64) << (8 * (i % 8));
            }
            let valid = api::executable_path(path);
            let reply = dispatch(&msg, |_, p| {
                assert!(valid);
                assert_eq!(p, Some(path));
                Some(IpcMsg::with_label(api::REPLY_OK).with_cap(0, CapabilityToken(456)))
            });
            assert_eq!(reply.label == api::REPLY_OK, valid);
        }
    }
}
