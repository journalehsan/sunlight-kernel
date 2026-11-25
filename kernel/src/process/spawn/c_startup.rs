use super::{copy_to_user, ExecStack, Process, SpawnError};
use crate::process::{elf_loader::handoff::Fault, layout::USER_STACK_TOP};
use x86_64::VirtAddr;

pub(in crate::process) const IMAGE: &[u8] = include_bytes!(concat!(
    env!("SUNLIGHT_DYNAMIC_PROBES"),
    "/elf-c-startup.sunapp/Program/probe"
));

const STACK_BYTES: usize = 288;
const ARGV0: &[u8] = b"c-startup\0";
const ARGV0_OFFSET: usize = 262;
const ARGV1_OFFSET: usize = 260;
const RANDOM_OFFSET: usize = 272;

pub(super) fn validate(
    process: &Process,
    hhdm: VirtAddr,
    stack: &mut ExecStack,
    auxv: &[(u64, u64)],
    fault: Option<Fault>,
) -> Result<(), SpawnError> {
    let bad_layout = SpawnError::ElfLoadFailed;
    let mutation: Option<(usize, &[u8])> = match fault {
        Some(Fault::CStartupArgc) => Some((0, &[0xff; 8])),
        Some(Fault::CStartupArgv) => Some((3 * 8, &[1][..])),
        Some(Fault::CStartupString) => Some((ARGV0_OFFSET + ARGV0.len() - 1, b"X")),
        Some(Fault::CStartupEnvp) => Some((4 * 8, &[1][..])),
        Some(Fault::CStartupAuxv) => Some((30 * 8, &[1][..])),
        Some(Fault::CStartupAlignment) => {
            stack.rsp += 8;
            None
        }
        _ => None,
    };
    if let Some((offset, bytes)) = mutation {
        copy_to_user(process, hhdm, stack.rsp + offset as u64, bytes)?;
    }

    if stack.rsp & 15 != 0
        || USER_STACK_TOP.checked_sub(stack.rsp) != Some(STACK_BYTES as u64)
        || stack.argv_ptr != stack.rsp + 8
        || stack.envp_ptr != stack.rsp + 32
        || auxv.len() != 10
    {
        return Err(bad_layout);
    }

    let mut snapshot = [0; STACK_BYTES];
    crate::memory::user::copy_from_process_bytes(process, hhdm, stack.rsp, &mut snapshot)
        .map_err(|_| bad_layout)?;
    let word =
        |index: usize| u64::from_le_bytes(snapshot[index * 8..(index + 1) * 8].try_into().unwrap());
    let argv0 = stack.rsp + ARGV0_OFFSET as u64;
    let argv1 = stack.rsp + ARGV1_OFFSET as u64;
    let random = stack.rsp + RANDOM_OFFSET as u64;
    let prefix = [2, argv0, argv1, 0, 0];
    let suffix = [31, argv0, 25, random, 0, 0];
    if prefix
        .into_iter()
        .enumerate()
        .any(|(i, value)| word(i) != value)
        || auxv
            .iter()
            .enumerate()
            .any(|(i, &(tag, value))| word(5 + 2 * i) != tag || word(6 + 2 * i) != value)
        || suffix
            .into_iter()
            .enumerate()
            .any(|(i, value)| word(25 + i) != value)
        || &snapshot[ARGV0_OFFSET..RANDOM_OFFSET] != ARGV0
        || !matches!(snapshot[ARGV1_OFFSET], b'P' | b'N' | b'A' | b'W' | b'L')
        || snapshot[ARGV1_OFFSET + 1] != 0
    {
        return Err(bad_layout);
    }

    if fault == Some(Fault::CStartupBeforeEntry) {
        crate::serial_println!(
            "[C-STARTUP] pre-entry stack validated argc=2 envc=0 context=unpublished"
        );
        return Err(SpawnError::NoMemory);
    }
    Ok(())
}
