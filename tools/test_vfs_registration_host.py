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
pub mod capability {
    pub struct CapabilityBroker { pub endpoints: Vec<(u32, usize)> }
    impl CapabilityBroker {
        pub fn endpoint_owner(&self, id: u32) -> Option<usize> {
            self.endpoints.iter().find(|(endpoint, _)| *endpoint == id).map(|(_, pid)| *pid)
        }
    }
}
pub mod sched {
    pub struct Foreground { pub incarnation: u64 }
    pub struct AddressSpace { pub generation: u64 }
    impl AddressSpace { pub fn identity(&self) -> &Self { self } }
    pub struct Process {
        pub pid: usize, pub trusted_vfs_service: bool, pub exiting: bool,
        pub uid: u32, pub gid: u32,
        pub foreground: Foreground, pub address_space: AddressSpace,
    }
    impl Process { pub fn is_exiting(&self) -> bool { self.exiting } }
    pub struct Scheduler { pub processes: Vec<Process> }
}
pub mod process {
    pub use crate::sched::Process;
}
#[cfg(test)]
mod ipc {
    #[derive(Debug, PartialEq, Eq)]
    pub enum IpcError { InvalidCapability }
    #[path = "ADAPTER_PATH"]
    mod vfs_registration;
    #[path = "RESOURCES_PATH"]
    mod vfs_resources;
    #[test]
    fn production_adapter_authenticates_registration_and_admission_identity() {
        use crate::capability::CapabilityBroker;
        use crate::sched::{AddressSpace, Foreground, Process, Scheduler};
        let mut sched = Scheduler { processes: vec![Process {
            pid: 3, trusted_vfs_service: false, exiting: false, uid: 0, gid: 0,
            foreground: Foreground { incarnation: 5 }, address_space: AddressSpace { generation: 7 },
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
        assert_eq!(vfs_registration::capture(3, &sched, &caps).unwrap().registration.owner.endpoint, 0);
        assert_eq!(vfs_registration::register(3, 8, &sched, &caps), denied);
        caps.endpoints.retain(|(id, _)| *id != 0);
        assert_eq!(vfs_registration::register(3, 8, &sched, &caps), Ok(()));
        assert_eq!(vfs_registration::register(3, 8, &sched, &caps), Ok(()));
        sched.processes.push(Process {
            pid: 4, trusted_vfs_service: true, exiting: false, uid: 0, gid: 0,
            foreground: Foreground { incarnation: 6 }, address_space: AddressSpace { generation: 8 },
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
            pid: 10, trusted_vfs_service: false, exiting: false, uid: 1001, gid: 1002,
            foreground: Foreground { incarnation: 11 }, address_space: AddressSpace { generation: 12 },
        });
        let snapshot = vfs_registration::capture(10, &sched, &caps).unwrap();
        assert_eq!(snapshot.caller.pid, 10);
        assert_eq!(snapshot.caller.incarnation, 11);
        assert_eq!(snapshot.caller.address_space_generation, 12);
        assert_eq!((snapshot.uid, snapshot.gid), (1001, 1002));
        assert!(snapshot.groups.is_empty());
        assert_eq!(snapshot.registration.owner.pid, 4);
        assert_eq!(vfs_registration::revalidate(snapshot, &sched, &caps), Ok(()));
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
            'edition = "2021"\n[workspace]\n[dependencies]\nspin = "0.9"\n'
            f'sunlight-ipc = {{ path = "{ROOT / "ipc"}" }}\n')
        (work / 'src/lib.rs').write_text(SOURCE.replace(
            'ADAPTER_PATH', str(ROOT / 'kernel/src/ipc/vfs_registration.rs')).replace(
            'RESOURCES_PATH', str(ROOT / 'kernel/src/ipc/vfs_resources.rs')))
        subprocess.run([
            'cargo', 'test', '--manifest-path', str(work / 'Cargo.toml'),
            '--target', 'x86_64-unknown-linux-gnu', '--target-dir', str(target), '--offline',
        ], cwd=ROOT, check=True, timeout=120)


if __name__ == '__main__':
    main()
