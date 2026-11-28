#!/usr/bin/env python3
"""Exercise VFS registration and admission snapshots with host process/broker records.

This tests admission and replacement decisions, not kernel locks, teardown,
scheduler continuations, or native IPC. Evidence stays under target/.
"""
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]

SOURCE = r'''
extern crate alloc;
// Mounted filesystems belong to the host harness, never kernel test boot code.
fn mounted_namespace() -> sunlight_vfs_server::directory::DirectoryNamespace {
    let mut fs = sunlight_fs::Vfs::new();
    fs.mount_tmpfs("/", sunlight_fs::Tmpfs::new(&[])).unwrap();
    sunlight_vfs_server::directory::DirectoryNamespace::mounted(fs).unwrap()
}
fn mounted_cwd_namespace(uid: u32, gid: u32) -> sunlight_vfs_server::directory::DirectoryNamespace {
    let namespace = mounted_namespace();
    {
        let shared = namespace.shared();
        let mut fs = shared.lock();
        fs.mkdir("/work", 0, 0, 0o755).unwrap();
        fs.mount_tmpfs("/work", sunlight_fs::Tmpfs::new(&[])).unwrap();
        fs.mkdir("/work/cwd", uid, gid, 0o755).unwrap();
        let h = fs.create_file_exclusive("/work/cwd/data", uid, gid, 0o644).unwrap();
        fs.close(h).unwrap();
    }
    namespace
}
fn downloads_namespaces() -> (sunlight_fs::Vfs, sunlight_vfs_server::directory::DirectoryNamespace) {
    use sunlight_fs::{RamEntry, Tmpfs, Vfs, vfs::mode};
    const SEED: &[RamEntry] = &[
        RamEntry::dir("/root", 0, 0, mode::DIR_755),
        RamEntry::dir("/root/Downloads", 0, 0, mode::DIR_700),
        RamEntry::dir("/etc", 0, 0, mode::DIR_755),
    ];
    let mut legacy = Vfs::new();
    legacy.reserve_service_downloads(SEED).unwrap();
    legacy.mount_tmpfs("/", Tmpfs::new(SEED)).unwrap();
    let mut service = Vfs::new();
    service.mount_tmpfs("/", Tmpfs::new(SEED)).unwrap();
    service.mount_service_downloads().unwrap();
    (legacy, sunlight_vfs_server::directory::DirectoryNamespace::mounted(service).unwrap())
}
pub mod timekeeping { pub const TICK_HZ: u64 = 100; }
pub mod capability {
    #[path = "DELIVERY_PATH"]
    pub mod delivery;
    pub struct CapabilityBroker { pub endpoints: Vec<(u32, usize)> }
    impl CapabilityBroker {
        pub fn endpoint_owner(&self, id: u32) -> Option<usize> {
            self.endpoints.iter().find(|(endpoint, _)| *endpoint == id).map(|(_, pid)| *pid)
        }
    }
}
pub mod sched {
    #[path = "RETIREMENT_PATH"]
    pub mod image_retirement;
    #[cfg(test)]
    pub(crate) use self::image_retirement::PrivateImageRetired;
    #[cfg(test)]
    pub(crate) fn exit_proof(
        image: crate::capability::delivery::RecipientIdentity,
        last_owner: bool,
        reclaimed: bool,
        fenced: bool,
    ) -> Option<PrivateImageRetired> {
        PrivateImageRetired::after_exit_cleanup(image, last_owner, reclaimed, fenced)
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ProcessState { Ready, Running, BlockedOnIpc }
    #[derive(Clone, Default)]
    pub struct Foreground {
        pub incarnation: u64, pub session_broker: u64, pub session: u64,
        pub generation: u64, pub member: u64, pub cancel: u8, pub forced_exit: Option<i32>,
    }
    impl Foreground { pub fn execution(&self) -> u64 { self.member } }
    #[derive(Clone)]
    pub struct AddressSpace { pub generation: u64 }
    impl AddressSpace { pub fn identity(&self) -> &Self { self } }
    #[derive(Clone)]
    pub struct Process {
        pub pid: usize, pub trusted_vfs_service: bool, pub exiting: bool,
        pub vfs_root_metadata: bool, pub vfs_boot_root: bool, pub vfs_file_routed: bool,
        pub vfs_root_rebased: bool, pub vfs_stat_routed: bool, pub vfs_downloads_cwd: bool,
        pub cwd: String, pub shared_owner: Option<usize>,
        pub uid: u32, pub gid: u32,
        pub state: ProcessState, pub pending_call: Option<()>,
        pub ipc_reply_target: Option<()>, pub ipc_endpoint: Option<u32>,
        pub foreground: Foreground, pub address_space: AddressSpace,
    }
    impl Process { pub fn is_exiting(&self) -> bool { self.exiting } }
    pub struct Scheduler {
        pub global_tick: u64, pub processes: Vec<Process>,
        pub(crate) pending_children: crate::process::pending_state::Pending<Box<Process>, 8>,
    }
    impl Scheduler {
        pub fn shared_process_index(&self, idx: usize) -> usize {
            self.processes[idx].shared_owner.unwrap_or(idx)
        }
    }
}
pub mod process {
    pub use crate::sched::{Process, ProcessState};
    #[path = "PENDING_STATE_PATH"]
    pub(crate) mod pending_state;
    #[path = "EXEC_RETIREMENT_PATH"]
    pub(crate) mod exec_retirement;
    pub(crate) fn exec_proof(
        image: crate::capability::delivery::RecipientIdentity,
        was_trusted: bool,
        still_trusted: bool,
        reclaimed: bool,
    ) -> Option<exec_retirement::PrivateVfsExecRetired> {
        exec_retirement::FencedVfsExec::after_withdrawal(image, was_trusted, still_trusted)?
            .after_reclaim(reclaimed)
    }
}
#[cfg(test)]
mod ipc {
    pub use sunlight_ipc::IpcMsg;
    #[derive(Debug, PartialEq, Eq)]
    pub enum IpcError { InvalidCapability }
    fn wake_terminal(sched: &mut crate::sched::Scheduler, idx: usize) {
        sched.processes[idx].state = crate::sched::ProcessState::Ready;
    }
    #[path = "ADAPTER_PATH"]
    mod vfs_registration;
    #[path = "RESOURCES_PATH"]
    mod vfs_resources;
    #[test]
    fn production_adapter_authenticates_registration_and_admission_identity() {
        use crate::capability::CapabilityBroker;
        use crate::sched::{AddressSpace, Foreground, Process, Scheduler};
        let mut sched = Scheduler { global_tick: 0, pending_children: crate::process::pending_state::Pending::new(), processes: vec![Process {
            pid: 3, trusted_vfs_service: false, exiting: false, vfs_root_metadata: false, vfs_boot_root: false, vfs_file_routed: false, cwd: String::from("/"),
            vfs_root_rebased: false, vfs_stat_routed: false, vfs_downloads_cwd: false, shared_owner: None,
            state: crate::sched::ProcessState::Running, pending_call: None,
            ipc_reply_target: None, ipc_endpoint: None, uid: 0, gid: 0,
            foreground: Foreground { incarnation: 5, ..Default::default() }, address_space: AddressSpace { generation: 7 },
        }] };
        let mut caps = CapabilityBroker { endpoints: vec![(0, 3), (8, 3), (9, 4)] };
        let denied = Err(IpcError::InvalidCapability);
        // A matching PID, endpoint and image name cannot replace boot provenance.
        assert_eq!(vfs_registration::register(3, 8, &sched, &caps), denied);
        sched.processes[0].trusted_vfs_service = true;
        assert_eq!(vfs_registration::capture(3, &sched, &caps), Err(IpcError::InvalidCapability));
        assert_eq!(vfs_registration::register(3, 9, &sched, &caps), denied);
        // Real boot can allocate VFS endpoint zero before init's endpoint.
        assert_eq!(vfs_registration::register(3, 0, &sched, &caps), Ok(()));
        let snapshot = vfs_registration::control(3,
            sunlight_ipc::vfs_rpc::retention::transport::SNAPSHOT, 0, &[], &sched, &caps)
            .unwrap().unwrap();
        assert_eq!(sunlight_ipc::vfs_rpc::retention::transport::decode_registration(
            &snapshot[..sunlight_ipc::vfs_rpc::retention::transport::REGISTRATION_BYTES])
            .unwrap().owner.endpoint, 0);
        assert_eq!(vfs_registration::capture(3, &sched, &caps).unwrap().registration.owner.endpoint, 0);
        assert_eq!(vfs_registration::register(3, 8, &sched, &caps), denied);
        caps.endpoints.retain(|(id, _)| *id != 0);
        assert_eq!(vfs_registration::register(3, 8, &sched, &caps), Ok(()));
        assert_eq!(vfs_registration::register(3, 8, &sched, &caps), Ok(()));
        sched.processes.push(Process {
            pid: 4, trusted_vfs_service: true, exiting: false, vfs_root_metadata: false, vfs_boot_root: false, vfs_file_routed: false, cwd: String::from("/"),
            vfs_root_rebased: false, vfs_stat_routed: false, vfs_downloads_cwd: false, shared_owner: None,
            state: crate::sched::ProcessState::Running, pending_call: None,
            ipc_reply_target: None, ipc_endpoint: None, uid: 0, gid: 0,
            foreground: Foreground { incarnation: 6, ..Default::default() }, address_space: AddressSpace { generation: 8 },
        });
        assert_eq!(vfs_registration::register(4, 9, &sched, &caps), denied);
        // Exiting the incumbent permits explicit replacement, never coexistence.
        sched.processes[0].exiting = true;
        assert_eq!(vfs_registration::register(3, 8, &sched, &caps), denied);
        assert_eq!(vfs_registration::register(4, 9, &sched, &caps), Ok(()));
        // A closed endpoint retires its registration even while the owner lives.
        caps.endpoints.retain(|(id, _)| *id != 9);
        sched.processes[0].exiting = false;
        assert_eq!(vfs_registration::register(3, 8, &sched, &caps), Ok(()));
        // Exec withdraws the trusted role and changes VM identity.
        sched.processes[0].trusted_vfs_service = false;
        sched.processes[0].address_space.generation += 1;
        assert_eq!(vfs_registration::register(3, 8, &sched, &caps), denied);
        caps.endpoints.push((9, 4));
        assert_eq!(vfs_registration::register(4, 9, &sched, &caps), Ok(()));
        // A normal caller needs no trusted service role. Its credentials come
        // from process records, independently of the service's credentials.
        sched.processes.push(Process {
            pid: 10, trusted_vfs_service: false, exiting: false, vfs_root_metadata: false, vfs_boot_root: false, vfs_file_routed: false, cwd: String::from("/"),
            vfs_root_rebased: false, vfs_stat_routed: false, vfs_downloads_cwd: false, shared_owner: None,
            state: crate::sched::ProcessState::Running, pending_call: None,
            ipc_reply_target: None, ipc_endpoint: None, uid: 1001, gid: 1002,
            foreground: Foreground { incarnation: 11, ..Default::default() }, address_space: AddressSpace { generation: 12 },
        });
        let snapshot = vfs_registration::capture(10, &sched, &caps).unwrap();
        assert_eq!(snapshot.caller.pid, 10);
        assert_eq!(snapshot.caller.incarnation, 11);
        assert_eq!(snapshot.caller.address_space_generation, 12);
        assert_eq!((snapshot.uid, snapshot.gid), (1001, 1002));
        assert!(snapshot.groups.is_empty());
        assert_eq!(snapshot.registration.owner.pid, 4);
        assert_eq!(vfs_registration::revalidate(snapshot, &sched, &caps), Ok(()));
        // Valid credentials alone do not install a namespace or admit a call.
        assert_eq!(vfs_registration::admit_stat(snapshot, "/", &sched, &caps),
            Err(vfs_registration::StatAdmissionError::Prepare(
                sunlight_ipc::vfs_rpc::pending_table::PrepareError::Resource(
                    vfs_resources::Error::StaleNamespace))));
        assert!(vfs_registration::reap_next().is_none());
        assert_eq!(vfs_registration::capture(99, &sched, &caps), Err(IpcError::InvalidCapability));
        for field in 0..6 {
            // Each independently invalidates the retained admission snapshot.
            match field {
                0 => sched.processes[2].uid += 1,
                1 => sched.processes[2].gid += 1,
                2 => sched.processes[2].foreground.incarnation += 1,
                3 => sched.processes[2].address_space.generation += 1,
                4 => sched.processes[2].exiting = true,
                _ => sched.processes[2].pid = 99,
            }
            assert_eq!(vfs_registration::revalidate(snapshot, &sched, &caps), denied);
            assert_eq!(vfs_registration::admit_stat(snapshot, "/", &sched, &caps),
                Err(vfs_registration::StatAdmissionError::Identity));
            sched.processes[2].uid = 1001;
            sched.processes[2].gid = 1002;
            sched.processes[2].foreground.incarnation = 11;
            sched.processes[2].address_space.generation = 12;
            sched.processes[2].exiting = false;
            sched.processes[2].pid = 10;
        }
        for field in 0..3 {
            match field {
                0 => sched.processes[2].pid = 0,
                1 => sched.processes[2].foreground.incarnation = 0,
                _ => sched.processes[2].address_space.generation = 0,
            }
            let pid = sched.processes[2].pid;
            assert_eq!(vfs_registration::capture(pid, &sched, &caps), Err(IpcError::InvalidCapability));
            sched.processes[2].pid = 10;
            sched.processes[2].foreground.incarnation = 11;
            sched.processes[2].address_space.generation = 12;
        }
        // UID/GID zero is valid but this API supplies no namespace grants.
        sched.processes[2].uid = 0;
        sched.processes[2].gid = 0;
        let root = vfs_registration::capture(10, &sched, &caps).unwrap();
        assert_eq!((root.uid, root.gid), (0, 0));
        sched.processes[2].uid = 1001;
        sched.processes[2].gid = 1002;
        // Every service-lifetime change retires the active binding. Restoring
        // identical records and registering again must still change the epoch.
        for field in 0..6 {
            let old = vfs_registration::capture(10, &sched, &caps).unwrap();
            match field {
                0 => sched.processes[1].trusted_vfs_service = false,
                1 => sched.processes[1].exiting = true,
                2 => sched.processes[1].foreground.incarnation += 1,
                3 => sched.processes[1].address_space.generation += 1,
                4 => caps.endpoints.retain(|(id, _)| *id != 9),
                _ => caps.endpoints.iter_mut().find(|(id, _)| *id == 9).unwrap().1 = 3,
            }
            assert_eq!(vfs_registration::capture(10, &sched, &caps), Err(IpcError::InvalidCapability));
            assert_eq!(vfs_registration::revalidate(old, &sched, &caps), denied);
            sched.processes[1].trusted_vfs_service = true;
            sched.processes[1].exiting = false;
            sched.processes[1].foreground.incarnation = 6;
            sched.processes[1].address_space.generation = 8;
            caps.endpoints.retain(|(id, _)| *id != 9);
            caps.endpoints.push((9, 4));
            // Capture cannot implicitly register a restored or new service.
            assert_eq!(vfs_registration::capture(10, &sched, &caps), Err(IpcError::InvalidCapability));
            assert_eq!(vfs_registration::register(4, 9, &sched, &caps), Ok(()));
            let new = vfs_registration::capture(10, &sched, &caps).unwrap();
            assert!(new.registration.epoch > old.registration.epoch);
            assert_eq!(vfs_registration::revalidate(old, &sched, &caps), denied);
            assert_eq!(vfs_registration::revalidate(new, &sched, &caps), Ok(()));
        }
        // Exercise the boot-lived entrypoints as well as the local-state tests.
        // Retirement must happen eagerly, even if process/broker records still
        // look live; capture must not silently resurrect the old epoch.
        let old = vfs_registration::capture(10, &sched, &caps).unwrap();
        vfs_registration::retire_process(old.caller);
        assert_eq!(vfs_registration::revalidate(old, &sched, &caps), Ok(()));
        let service = vfs_registration::process_identity(&sched.processes[1]);
        vfs_registration::retire_endpoint(8, service);
        assert_eq!(vfs_registration::revalidate(old, &sched, &caps), Ok(()));
        vfs_registration::retire_endpoint(9, service);
        assert_eq!(vfs_registration::capture(10, &sched, &caps), Err(IpcError::InvalidCapability));
        vfs_registration::register(4, 9, &sched, &caps).unwrap();
        let new = vfs_registration::capture(10, &sched, &caps).unwrap();
        assert!(new.registration.epoch > old.registration.epoch);
        assert_eq!(vfs_registration::revalidate(old, &sched, &caps), denied);
        vfs_registration::retire_process(service);
        vfs_registration::retire_process(service);
        assert_eq!(vfs_registration::capture(10, &sched, &caps), Err(IpcError::InvalidCapability));
    }
}
'''


def main():
    target = ROOT / 'target/vfs-registration-host'
    target.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='vfs-registration-') as directory:
        work = Path(directory)
        (work / 'src').mkdir()
        (work / 'Cargo.toml').write_text(
            '[package]\nname = "vfs-registration-host"\nversion = "0.0.0"\n'
            'edition = "2021"\n[workspace]\n[features]\nvfs_c2e_candidate = []\n[dependencies]\nspin = "0.9"\n'
            f'sunlight-ipc = {{ path = "{ROOT / "ipc"}" }}\n'
            f'sunlight-fs = {{ path = "{ROOT / "sunlight-fs"}" }}\n'
            f'sunlight-mm = {{ path = "{ROOT / "sunlight-mm"}", features = ["testutil"] }}\n'
            f'sunlight-vfs-server = {{ path = "{ROOT / "services/vfs_server"}", features = ["vfs-c2e-candidate"] }}\n'
            f'sunlight-compat-linux = {{ path = "{ROOT / "compat-linux"}" }}\n')
        (work / 'src/lib.rs').write_text(SOURCE.replace(
            'ADAPTER_PATH', str(ROOT / 'kernel/src/ipc/vfs_registration.rs')).replace(
            'RESOURCES_PATH', str(ROOT / 'kernel/src/ipc/vfs_resources.rs')).replace(
            'DELIVERY_PATH', str(ROOT / 'kernel/src/capability/delivery.rs')).replace(
            'PENDING_STATE_PATH', str(ROOT / 'kernel/src/process/pending_state.rs')).replace(
            'EXEC_RETIREMENT_PATH', str(ROOT / 'kernel/src/process/exec_retirement.rs')).replace(
            'RETIREMENT_PATH', str(ROOT / 'kernel/src/sched/image_retirement.rs')))
        subprocess.run([
            'cargo', 'test', '--manifest-path', str(work / 'Cargo.toml'),
            '--target', 'x86_64-unknown-linux-gnu', '--target-dir', str(target), '--offline',
            '--features', 'vfs_c2e_candidate',
        ], cwd=ROOT, check=True, timeout=120)


if __name__ == '__main__':
    main()
