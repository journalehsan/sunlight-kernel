use super::*;
use sunlight_fs::{mode, FileSystem, Tmpfs, TmpfsLimits};
use sunlight_ipc::vfs_rpc::{
    registration::EndpointOwner,
    retention::ownership::{Ownership, Retained},
};

fn registration() -> Registration {
    Registration {
        owner: EndpointOwner {
            endpoint: 0,
            pid: 2,
            incarnation: 3,
            address_space_generation: 4,
        },
        epoch: 5,
    }
}
fn scope() -> Scope {
    Scope {
        caller: Caller {
            pid: 6,
            incarnation: 7,
            address_space_generation: 8,
        },
        namespace_id: 9,
        namespace_generation: 10,
    }
}
fn namespace() -> DirectoryNamespace {
    let mut fs = Tmpfs::with_limits(&[], TmpfsLimits::new(0, 4)).unwrap();
    fs.mkdir("/root", 1, 2, mode::DIR_755).unwrap();
    fs.mkdir("/root/cwd", 3, 4, mode::DIR_700).unwrap();
    DirectoryNamespace::new(fs).unwrap()
}
fn retain(registration: Registration, scope: Scope, id: u64) -> [u8; wire::FRAME_BYTES] {
    wire::PendingRetain::new(registration, scope, id)
        .unwrap()
        .request()
        .encode()
        .unwrap()
}
fn acquire<const P: usize, const L: usize, const R: usize>(
    runtime: &mut Runtime<P, L, R>,
    r: Registration,
    s: Scope,
    id: u64,
) -> Receipt {
    let wire::Frame::Retained(receipt) = runtime.request(r, &retain(r, s, id)).unwrap() else {
        panic!()
    };
    receipt
}

#[test]
fn binding_is_transactional_idempotent_and_never_resets_an_epoch() {
    let r = registration();
    let s = scope();
    let mut runtime = Runtime::<1, 2, 2>::new();
    assert_eq!(runtime.active(), None);
    assert_eq!(runtime.request(r, &retain(r, s, 1)), Err(Error::Identity));
    assert_eq!(
        runtime.bind(Registration {
            owner: EndpointOwner { pid: 0, ..r.owner },
            ..r
        }),
        Err(Error::Identity)
    );
    assert_eq!(runtime.active(), None);
    runtime.bind(r).unwrap();
    runtime
        .install(r, s, &namespace(), "/root", "/root/cwd")
        .unwrap();
    let receipt = acquire(&mut runtime, r, s, 1);
    runtime.bind(r).unwrap();
    assert!(runtime.metadata(r, receipt).is_ok());
    assert_eq!(
        runtime.request(r, &retain(r, s, 1)),
        Err(Error::Retention(
            sunlight_ipc::vfs_rpc::retention::Error::Replay
        ))
    );
    let next = Registration {
        epoch: r.epoch + 1,
        ..r
    };
    assert_eq!(runtime.bind(next), Err(Error::Identity));
    runtime.retire_registration(r).unwrap();
    assert_eq!(runtime.bind(r), Err(Error::Identity));
    assert_eq!(
        runtime.install(r, s, &namespace(), "/root", "/root/cwd"),
        Err(Error::Identity)
    );
    runtime.bind(next).unwrap();
    runtime.retire_registration(r).unwrap(); // stale repeat cannot retire replacement
    assert_eq!(runtime.active(), Some(next));
    assert!(runtime.metadata(r, receipt).is_ok());
}

#[test]
fn caller_retirement_matches_every_identity_field_and_preserves_lease_pins() {
    let r = registration();
    let s = scope();
    let mut runtime = Runtime::<2, 5, 1>::new();
    runtime.bind(r).unwrap();
    runtime
        .install(r, s, &namespace(), "/root", "/root/cwd")
        .unwrap();
    let receipt = acquire(&mut runtime, r, s, 1);
    for field in 0..3 {
        let mut caller = s.caller;
        match field {
            0 => caller.pid += 1,
            1 => caller.incarnation += 1,
            _ => caller.address_space_generation += 1,
        }
        runtime.retire_caller(caller);
        acquire(&mut runtime, r, s, field + 2);
    }
    runtime.retire_caller(s.caller);
    runtime.retire_caller(s.caller);
    assert!(matches!(
        runtime.request(r, &retain(r, s, 5)).unwrap(),
        wire::Frame::Refused {
            reason: wire::Refusal::Resolve,
            ..
        }
    ));
    assert!(runtime.metadata(r, receipt).is_ok());
    let replacement = Scope {
        caller: Caller {
            incarnation: s.caller.incarnation + 1,
            ..s.caller
        },
        namespace_generation: s.namespace_generation + 1,
        ..s
    };
    runtime
        .install(r, replacement, &namespace(), "/root", "/root/cwd")
        .unwrap();
    runtime.retire_caller(s.caller);
    acquire(&mut runtime, r, replacement, 6);
    runtime
        .request(r, &wire::Frame::Release(receipt).encode().unwrap())
        .unwrap();
    assert_eq!(runtime.metadata(r, receipt), Err(Error::Scope));
}

#[test]
fn retirement_authenticates_full_registration_and_late_release_survives_replacement() {
    let r = registration();
    let s = scope();
    let mut runtime = Runtime::<1, 2, 2>::new();
    runtime.bind(r).unwrap();
    runtime
        .install(r, s, &namespace(), "/root", "/root/cwd")
        .unwrap();
    let mut owner = Ownership::<1>::new(r).unwrap();
    let request = owner.prepare(s).unwrap().encode().unwrap();
    let reply = runtime.request(r, &request).unwrap().encode().unwrap(); // reply delayed
    for field in 0..5 {
        let mut wrong = r;
        match field {
            0 => wrong.owner.endpoint += 1,
            1 => wrong.owner.pid += 1,
            2 => wrong.owner.incarnation += 1,
            3 => wrong.owner.address_space_generation += 1,
            _ => wrong.epoch += 1,
        }
        assert_eq!(runtime.retire_registration(wrong), Err(Error::Identity));
        assert_eq!(runtime.request(wrong, &request), Err(Error::Identity));
        assert_eq!(runtime.active(), Some(r));
    }
    runtime.retire_registration(r).unwrap();
    let next = Registration {
        epoch: r.epoch + 1,
        ..r
    };
    runtime.bind(next).unwrap();
    runtime
        .install(next, s, &namespace(), "/root", "/root/cwd")
        .unwrap();
    let new_receipt = acquire(&mut runtime, next, s, 1);
    owner.abandon_all();
    let Retained::Release(release) = owner.retained(1, r, &reply).unwrap() else {
        panic!()
    };
    let bytes = release.encode().unwrap();
    assert_eq!(runtime.request(next, &bytes), Err(Error::Identity));
    let released = runtime.request(r, &bytes).unwrap(); // lose acknowledgement
    assert_eq!(runtime.request(r, &bytes), Ok(released));
    owner.released(1, r, &released.encode().unwrap()).unwrap();
    assert_eq!(owner.outstanding(), 0);
    assert!(runtime.metadata(next, new_receipt).is_ok());
    assert!(matches!(
        runtime.request(r, &retain(r, s, 2)).unwrap(),
        wire::Frame::Refused {
            reason: wire::Refusal::Resolve,
            ..
        }
    ));
    assert_eq!(
        runtime.request(r, &request),
        Err(Error::Retention(
            sunlight_ipc::vfs_rpc::retention::Error::Replay
        ))
    );
}

#[test]
fn retired_empty_epochs_keep_replay_and_ack_state_and_capacity_fails_closed() {
    let r = registration();
    let s = scope();
    let mut runtime = Runtime::<1, 1, 2>::new();
    for epoch in r.epoch..r.epoch + 2 {
        let current = Registration { epoch, ..r };
        runtime.bind(current).unwrap();
        runtime
            .install(current, s, &namespace(), "/root", "/root/cwd")
            .unwrap();
        let receipt = acquire(&mut runtime, current, s, 1);
        runtime.retire_registration(current).unwrap();
        let release = wire::Frame::Release(receipt).encode().unwrap();
        let ack = runtime.request(current, &release).unwrap();
        assert_eq!(runtime.request(current, &release), Ok(ack));
    }
    assert_eq!(
        runtime.bind(Registration {
            epoch: r.epoch + 2,
            ..r
        }),
        Err(Error::Capacity)
    );
    assert_eq!(runtime.active(), None);
    assert_eq!(runtime.last_epoch, r.epoch + 1);
    assert_eq!(
        runtime.request(r, &retain(r, s, 1)),
        Err(Error::Retention(
            sunlight_ipc::vfs_rpc::retention::Error::Replay
        ))
    );
    assert_eq!(Runtime::<0, 0, 0>::new().bind(r), Err(Error::Capacity));
}

#[test]
fn direct_policy_retirement_is_permanent_and_preserves_empty_replay_state() {
    let r = registration();
    let s = scope();
    let mut policy = Policy::<1, 1>::new(r).unwrap();
    policy
        .install(s, &namespace(), "/root", "/root/cwd")
        .unwrap();
    policy.retire();
    policy.retire();
    assert_eq!(
        policy.install(
            Scope {
                namespace_generation: 11,
                ..s
            },
            &namespace(),
            "/root",
            "/root/cwd"
        ),
        Err(Error::Retired)
    );
    assert!(matches!(
        policy.request(r, &retain(r, s, 1)).unwrap(),
        wire::Frame::Refused {
            reason: wire::Refusal::Resolve,
            ..
        }
    ));
    assert_eq!(
        policy.request(r, &retain(r, s, 1)),
        Err(Error::Retention(
            sunlight_ipc::vfs_rpc::retention::Error::Replay
        ))
    );
}

#[test]
fn observing_replacement_preserves_retired_rows_and_fails_closed_at_capacity() {
    let old = registration();
    let next = Registration {
        epoch: old.epoch + 1,
        ..old
    };
    let mut service = Runtime::<1, 1, 1>::new();
    service.bind(old).unwrap();
    assert_eq!(
        service.observe_active(Registration {
            owner: EndpointOwner {
                pid: 0,
                ..next.owner
            },
            ..next
        }),
        Err(Error::Identity)
    );
    assert_eq!(service.active(), Some(old));
    assert_eq!(service.observe_active(next), Err(Error::Capacity));
    assert_eq!(service.active(), None);
    assert_eq!(service.registrations(), [Some(old)]);
    assert_eq!(service.observe_active(old), Err(Error::Identity));
    assert_eq!(service.observe_active(next), Err(Error::Capacity));
    let mut service = Runtime::<1, 1, 2>::new();
    service.observe_active(old).unwrap();
    service.observe_active(next).unwrap();
    service.observe_active(next).unwrap();
    assert_eq!(service.registrations(), [Some(old), Some(next)]);
    assert_eq!(service.observe_active(old), Err(Error::Identity));
    assert_eq!(service.active(), Some(next));
}

#[test]
fn control_poll_does_not_treat_missing_or_wrong_owner_status_as_evidence() {
    use super::control::Control;
    struct Channel {
        status: Result<(Registration, bool), ()>,
        reads: usize,
    }
    impl Control for Channel {
        fn snapshot(&mut self) -> Option<Registration> {
            None
        }
        fn status(&mut self, _: u64) -> Result<(Registration, bool), ()> {
            self.status
        }
        fn next(&mut self, _: u64) -> Result<Option<[u8; wire::FRAME_BYTES]>, ()> {
            self.reads += 1;
            Ok(None)
        }
        fn reply(&mut self, _: u64, _: &[u8; wire::FRAME_BYTES]) -> bool {
            panic!()
        }
    }
    let r = registration();
    let mut service = Runtime::<1, 1, 1>::new();
    service.bind(r).unwrap();
    let mut channel = Channel {
        status: Err(()),
        reads: 0,
    };
    service.poll_control(&mut channel);
    assert_eq!(channel.reads, 0);
    assert_eq!(service.active(), Some(r));
    channel.status = Ok((
        Registration {
            owner: EndpointOwner {
                incarnation: r.owner.incarnation + 1,
                ..r.owner
            },
            ..r
        },
        true,
    ));
    service.poll_control(&mut channel);
    assert_eq!(channel.reads, 0);
    assert_eq!(service.active(), Some(r));
    channel.status = Ok((r, true));
    service.poll_control(&mut channel);
    assert_eq!(channel.reads, 1);
    assert_eq!(service.active(), None);
    assert_eq!(service.registrations(), [Some(r)]);
}

#[test]
fn copied_lifecycle_route_requires_full_origin_and_preserves_leases_and_new_incarnations() {
    let r = registration();
    let s = scope();
    let mut runtime = Runtime::<2, 4, 1>::new();
    runtime.bind(r).unwrap();
    runtime
        .install(r, s, &namespace(), "/root", "/root/cwd")
        .unwrap();
    let receipt = acquire(&mut runtime, r, s, 1);
    let event = wire::Frame::CallerRetired {
        registration: r,
        caller: s.caller,
        event_id: 1,
    };
    let bytes = event.encode().unwrap();
    // The ordinary lease router has no lifecycle authority.
    assert!(runtime.request(r, &bytes).is_err());
    for field in 0..5 {
        let mut wrong = r;
        match field {
            0 => wrong.owner.endpoint += 1,
            1 => wrong.owner.pid += 1,
            2 => wrong.owner.incarnation += 1,
            3 => wrong.owner.address_space_generation += 1,
            _ => wrong.epoch += 1,
        }
        assert_eq!(runtime.control_request(wrong, &bytes), Err(Error::Identity));
    }
    acquire(&mut runtime, r, s, 2);
    let ack = wire::Frame::CallerRetiredAck {
        registration: r,
        caller: s.caller,
        event_id: 1,
    };
    assert_eq!(runtime.control_request(r, &bytes), Ok(ack));
    assert_eq!(runtime.control_request(r, &bytes), Ok(ack));
    assert!(runtime.metadata(r, receipt).is_ok());
    assert!(matches!(
        runtime.request(r, &retain(r, s, 3)).unwrap(),
        wire::Frame::Refused {
            reason: wire::Refusal::Resolve,
            ..
        }
    ));
    let next = Scope {
        caller: Caller {
            incarnation: s.caller.incarnation + 1,
            ..s.caller
        },
        namespace_generation: s.namespace_generation + 1,
        ..s
    };
    runtime
        .install(r, next, &namespace(), "/root", "/root/cwd")
        .unwrap();
    runtime.control_request(r, &bytes).unwrap();
    acquire(&mut runtime, r, next, 4);
    runtime
        .request(r, &wire::Frame::Release(receipt).encode().unwrap())
        .unwrap();
    assert_eq!(runtime.metadata(r, receipt), Err(Error::Scope));
}

struct Authority {
    namespace: DirectoryNamespace,
    registration: Registration,
    scope: Scope,
    live: bool,
    checks: usize,
}

impl ScopeAuthority for Authority {
    fn resolve(&mut self) -> Result<ProvisionedScope<'_>, Error> {
        Ok(ProvisionedScope {
            registration: self.registration,
            scope: self.scope,
            namespace: &self.namespace,
            root: "/root",
            directory: "/root/cwd",
        })
    }

    fn revalidate(&mut self, registration: Registration, scope: Scope) -> bool {
        self.checks += 1;
        assert_eq!(registration, self.registration);
        assert_eq!(scope, self.scope);
        self.live
    }
}

#[test]
fn provisioning_revalidates_and_withdraws_raced_policy_without_reusing_generation() {
    let r = registration();
    let s = scope();
    let mut runtime = Runtime::<1, 2, 1>::new();
    runtime.bind(r).unwrap();
    let mut authority = Authority {
        namespace: namespace(),
        registration: r,
        scope: s,
        live: false,
        checks: 0,
    };
    assert_eq!(runtime.provision(&mut authority), Err(Error::Identity));
    assert_eq!(authority.checks, 1);
    assert!(matches!(
        runtime.request(r, &retain(r, s, 1)).unwrap(),
        wire::Frame::Refused {
            reason: wire::Refusal::Resolve,
            ..
        }
    ));
    authority.live = true;
    assert_eq!(runtime.provision(&mut authority), Err(Error::Generation));
    assert_eq!(authority.checks, 1);
    authority.scope.namespace_generation += 1;
    assert_eq!(runtime.provision(&mut authority), Ok(authority.scope));
    assert_eq!(authority.checks, 2);
    let receipt = acquire(&mut runtime, r, authority.scope, 2);
    runtime.withdraw(r, authority.scope).unwrap();
    assert!(runtime.metadata(r, receipt).is_ok());
    runtime
        .request(r, &wire::Frame::Release(receipt).encode().unwrap())
        .unwrap();
    assert_eq!(runtime.metadata(r, receipt), Err(Error::Scope));
}

#[test]
fn provisioning_rejects_foreign_registration_before_publishing_pins() {
    let r = registration();
    let mut runtime = Runtime::<1, 1, 1>::new();
    runtime.bind(r).unwrap();
    let mut authority = Authority {
        namespace: namespace(),
        registration: Registration {
            epoch: r.epoch + 1,
            ..r
        },
        scope: scope(),
        live: true,
        checks: 0,
    };
    assert_eq!(runtime.provision(&mut authority), Err(Error::Identity));
    assert_eq!(authority.checks, 0);
    authority.registration = r;
    assert_eq!(runtime.provision(&mut authority), Ok(authority.scope));
    acquire(&mut runtime, r, authority.scope, 1);
}

fn stat_reply<const L: usize>(
    runtime: &Runtime<1, L, 1>,
    receipt: Receipt,
    path: &str,
    uid: u32,
    gid: u32,
) -> (u64, Option<sunlight_ipc::vfs_rpc::response::Metadata>) {
    use sunlight_ipc::vfs_rpc::{
        codec::Request,
        context::{Context, Dispatch, Namespace, LOOKUP_METADATA},
        response::{Reply, ReplyContract},
        stat_transport as api, Completion,
    };
    let scope = receipt.scope();
    let dispatch = Dispatch {
        request_id: 71,
        context: Context {
            caller: scope.caller,
            service: receipt.registration().service(),
            namespace: Namespace {
                id: scope.namespace_id,
                generation: scope.namespace_generation,
                root: receipt.generation(),
                directory: receipt.generation(),
            },
            uid,
            gid,
            groups: &[],
            grants: LOOKUP_METADATA,
        },
        request: Request::Stat { path },
    };
    let mut input = [0; api::INPUT_BYTES];
    let length = dispatch.encode(&mut input).unwrap();
    let bytes = runtime
        .stat(receipt.registration(), &input[..length])
        .unwrap();
    let reply = api::decode_reply(&bytes).unwrap();
    let metadata = if reply.words[2] == 0 {
        let Reply::Metadata(metadata) = ReplyContract::for_request(Request::Stat { path })
            .unwrap()
            .validate(
                Completion {
                    errno: 0,
                    output_bytes: 56,
                },
                reply.output,
            )
            .unwrap()
        else {
            panic!()
        };
        Some(metadata)
    } else {
        assert!(reply.output.is_empty());
        None
    };
    (reply.words[2], metadata)
}

#[test]
fn authenticated_stat_traverses_retained_cwd_and_encodes_search_and_path_errors() {
    let mut fs = sunlight_fs::Vfs::new();
    fs.mount_tmpfs("/", Tmpfs::new(&[])).unwrap();
    fs.mkdir("/jail", 42, 42, 0o755).unwrap();
    fs.mkdir("/jail/cwd", 42, 42, 0o710).unwrap();
    let h = fs
        .create_file_exclusive("/jail/cwd/data", 42, 42, 0)
        .unwrap();
    fs.write(h, 0, b"data").unwrap();
    fs.close(h).unwrap();
    let namespace = DirectoryNamespace::mounted(fs).unwrap();
    let shared = namespace.shared();
    let r = registration();
    let s = scope();
    let mut runtime = Runtime::<1, 2, 1>::new();
    runtime.bind(r).unwrap();
    runtime
        .install(r, s, &namespace, "/jail", "/jail/cwd")
        .unwrap();
    let receipt = acquire(&mut runtime, r, s, 1);
    let absolute = stat_reply(&runtime, receipt, "/cwd/data", 42, 42);
    assert_eq!(absolute, stat_reply(&runtime, receipt, "data", 43, 42));
    assert_eq!(absolute.1.unwrap().size, 4);
    assert_eq!(stat_reply(&runtime, receipt, "data", 43, 43).0, 13);
    assert_eq!(stat_reply(&runtime, receipt, "missing", 42, 42).0, 2);
    assert_eq!(stat_reply(&runtime, receipt, "data/..", 42, 42).0, 20);
    assert_eq!(stat_reply(&runtime, receipt, "data/", 42, 42).0, 20);
    shared.lock().rename("/jail/cwd", "/jail/moved").unwrap();
    assert_eq!(absolute, stat_reply(&runtime, receipt, "data", 42, 42));
    assert_eq!(stat_reply(&runtime, receipt, "/cwd/data", 42, 42).0, 2);
    let root = stat_reply(&runtime, receipt, "/", 42, 42);
    assert_eq!(root, stat_reply(&runtime, receipt, "../../..", 42, 42));
    runtime.withdraw(r, s).unwrap();
    assert_eq!(stat_reply(&runtime, receipt, "data", 42, 42).0, 5);
}

mod cwd_transition;
