//! Native fault injection, compiled only into the dedicated regression image.
use sunlight_ipc::{vfs_rpc::stat_transport as api, IpcMsg};

#[derive(Default)]
pub struct Probe {
    mode: u64,
    epoch: u64,
    held: bool,
    late: Option<[u8; api::REPLY_BYTES]>,
}

impl Probe {
    pub fn arm(&mut self, msg: &IpcMsg) -> Option<IpcMsg> {
        if msg.label != 0x53544641 {
            return None;
        }
        if msg.word_count == 1 && msg.cap_count == 0 {
            match msg.words[0] {
                5 => {
                    return Some(
                        IpcMsg::with_label(msg.label).word(0, u64::from(self.late.is_some())),
                    )
                }
                6 | 7 => {
                    self.held = false;
                    let late = self.late.take().expect("held caller reply");
                    // Use the active epoch captured with the held reply.
                    assert_eq!(api::complete(self.epoch, &late), msg.words[0] == 7);
                    return Some(IpcMsg::with_label(msg.label).word(0, 0));
                }
                _ => {}
            }
        }
        // The caller's epoch-loss wake can run before the failed NEXT syscall
        // returns here. An ordinary IPC acknowledgement synchronizes the gate
        // with the service's post-fault diagnostic and continued legacy loop.
        if msg.word_count == 1 && msg.cap_count == 0 && msg.words[0] == 3 {
            return Some(IpcMsg::with_label(msg.label).word(
                0,
                if self.mode == 0 && self.late.is_none() {
                    0
                } else {
                    1
                },
            ));
        }
        let valid = msg.word_count == 1
            && msg.cap_count == 0
            && matches!(msg.words[0], 1 | 2 | 4)
            && self.mode == 0
            && self.late.is_none();
        if valid {
            self.mode = msg.words[0];
        }
        Some(IpcMsg::with_label(msg.label).word(0, if valid { 0 } else { 1 }))
    }

    pub fn fail_pull(&mut self, epoch: u64) -> bool {
        if self.mode != 2 {
            return false;
        }
        // A real consumed pull followed by a failed user copy retires this
        // authenticated epoch. EMPTY must leave the injection armed.
        let result = api::fault_next_copy(epoch);
        if result != api::EMPTY {
            // Native user-copy failures use the legacy all-ones sentinel;
            // Linux-compatible entrypoints translate them to EFAULT instead.
            assert_eq!(result, u64::MAX);
            self.mode = 0;
            sunlight_ipc::debug_log(
                "[VFS-RPC] native consumed pull copy fault retired service epoch",
            );
        }
        true
    }

    pub fn withhold(&mut self, epoch: u64, reply: &[u8; api::REPLY_BYTES]) -> bool {
        if !self.held {
            if let Some(late) = self.late.take() {
                assert!(!api::complete(epoch, &late));
                sunlight_ipc::debug_log("[VFS-RPC] native late stat reply rejected after timeout");
            }
        }
        if self.mode == 1 || self.mode == 4 {
            self.epoch = epoch;
            self.held = self.mode == 4;
            self.mode = 0;
            self.late = Some(*reply);
            return true;
        }
        false
    }
}
