use super::*;
use sunlight_fs::{Credential, FsError, Vfs};

const OWNER: Credential = Credential { uid: 42, gid: 42 };

fn fixture<const P: usize>() -> (Runtime<P, 8, 1>, DirectoryNamespace) {
    let mut fs = Vfs::new();
    fs.mount_tmpfs("/", Tmpfs::new(&[])).unwrap();
    for path in ["/jail", "/jail/cwd", "/jail/cwd/next", "/jail/mount"] {
        fs.mkdir(path, 42, 42, 0o755).unwrap();
    }
    fs.chmod("/jail/cwd/next", 0o700).unwrap();
    fs.mount_tmpfs("/jail/mount", Tmpfs::new(&[])).unwrap();
    let namespace = DirectoryNamespace::mounted(fs).unwrap();
    let mut runtime = Runtime::new();
    runtime.bind(registration()).unwrap();
    runtime
        .install(registration(), scope(), &namespace, "/jail", "/jail/cwd")
        .unwrap();
    (runtime, namespace)
}

fn next(generation: u64) -> Scope {
    Scope {
        namespace_generation: generation,
        ..scope()
    }
}

fn child(generation: u64) -> Scope {
    Scope {
        caller: Caller {
            pid: 20,
            incarnation: 21,
            address_space_generation: 22,
        },
        ..next(generation)
    }
}

fn release<const P: usize>(runtime: &mut Runtime<P, 8, 1>, receipt: Receipt) {
    runtime
        .request(
            registration(),
            &wire::Frame::Release(receipt).encode().unwrap(),
        )
        .unwrap();
}

#[test]
fn narrowed_root_resets_cwd_clamps_dotdot_and_preserves_old_lease() {
    let (mut runtime, namespace) = fixture::<1>();
    let r = registration();
    let old = acquire(&mut runtime, r, scope(), 1);
    let destination = stat_reply(&runtime, old, "next", 42, 42);
    let shared = namespace.shared();
    shared.lock().rename("/jail/cwd", "/jail/moved").unwrap();
    shared.lock().mkdir("/jail/cwd", 1, 1, 0o755).unwrap();
    runtime
        .change_root(r, scope(), next(11), "next", OWNER, || true)
        .unwrap();
    let new = acquire(&mut runtime, r, next(11), 2);
    assert_eq!(stat_reply(&runtime, new, "/", 42, 42), destination);
    assert_eq!(stat_reply(&runtime, new, ".", 42, 42), destination);
    assert_eq!(stat_reply(&runtime, new, "../../..", 42, 42), destination);
    assert_eq!(runtime.policy_mut(r).unwrap().cwd(next(11)).unwrap(), "/");
    assert_eq!(stat_reply(&runtime, new, "/cwd", 42, 42).0, 2);
    assert_eq!(stat_reply(&runtime, old, ".", 42, 42).0, 5);
    assert!(runtime.metadata(r, old).is_ok());
    assert_eq!(runtime.withdraw(r, scope()), Err(Error::Scope));
    assert_eq!(shared.lock().open_handle_count(), 4);
    release(&mut runtime, old);
    assert_eq!(shared.lock().open_handle_count(), 2);
    runtime.retire_caller(scope().caller);
    release(&mut runtime, new);
    assert_eq!(shared.lock().open_handle_count(), 0);
}

#[test]
fn root_failures_preserve_policy_consume_generation_and_drop_partial_pins() {
    let (mut runtime, namespace) = fixture::<1>();
    let r = registration();
    let old = acquire(&mut runtime, r, scope(), 1);
    for (generation, path, caller, error) in [
        (11, "missing", OWNER, FsError::NotFound),
        (
            12,
            "next",
            Credential { uid: 43, gid: 43 },
            FsError::PermissionDenied,
        ),
    ] {
        assert_eq!(
            runtime.change_root(r, scope(), next(generation), path, caller, || panic!()),
            Err(Error::Filesystem(error))
        );
        assert_eq!(
            runtime.change_root(r, scope(), next(generation), ".", OWNER, || true),
            Err(Error::Generation)
        );
        assert_eq!(stat_reply(&runtime, old, ".", 42, 42).0, 0);
    }
    // Leave only one free handle: the second provisional open must fail and
    // release the first, preserving both old anchors.
    let shared = namespace.shared();
    let mut handles = alloc::vec::Vec::new();
    while let Ok(h) = shared.lock().open("/jail") {
        handles.push(h);
    }
    shared.lock().close(handles.pop().unwrap()).unwrap();
    let count = shared.lock().open_handle_count();
    assert!(matches!(
        runtime.change_root(r, scope(), next(13), "next", OWNER, || panic!()),
        Err(Error::Filesystem(_))
    ));
    assert_eq!(shared.lock().open_handle_count(), count);
    for h in handles {
        shared.lock().close(h).unwrap();
    }
    assert_eq!(
        runtime.change_root(r, scope(), next(14), "next", OWNER, || false),
        Err(Error::Identity)
    );
    assert_eq!(shared.lock().open_handle_count(), 2);
    assert_eq!(stat_reply(&runtime, old, ".", 42, 42).0, 5);
    release(&mut runtime, old);
    assert_eq!(shared.lock().open_handle_count(), 0);
}

#[test]
fn root_transition_crosses_mount_and_rejects_stale_scopes_and_epochs() {
    let (mut runtime, namespace) = fixture::<1>();
    let r = registration();
    let wrong = Registration {
        epoch: r.epoch + 1,
        ..r
    };
    assert_eq!(
        runtime.change_root(wrong, scope(), next(11), ".", OWNER, || panic!()),
        Err(Error::Identity)
    );
    assert_eq!(
        runtime.change_root(r, scope(), child(11), ".", OWNER, || panic!()),
        Err(Error::Scope)
    );
    let foreign_namespace = Scope {
        namespace_id: 100,
        ..next(11)
    };
    assert_eq!(
        runtime.change_root(r, scope(), foreign_namespace, ".", OWNER, || panic!()),
        Err(Error::Scope)
    );
    runtime
        .change_root(r, scope(), next(11), "/mount", OWNER, || true)
        .unwrap();
    let lease = acquire(&mut runtime, r, next(11), 1);
    let metadata = runtime.metadata(r, lease).unwrap();
    assert_eq!(metadata[0], metadata[1]);
    assert_eq!(runtime.policy_mut(r).unwrap().cwd(next(11)).unwrap(), "/");
    assert_eq!(
        runtime.change_root(r, scope(), next(12), ".", OWNER, || panic!()),
        Err(Error::Scope)
    );
    runtime.retire_registration(r).unwrap();
    assert_eq!(
        runtime.change_root(r, next(11), next(12), ".", OWNER, || panic!()),
        Err(Error::Identity)
    );
    release(&mut runtime, lease);
    assert_eq!(namespace.shared().lock().open_handle_count(), 0);
}

#[test]
fn derived_caller_keeps_exact_renamed_objects_and_independent_lifecycle() {
    let (mut runtime, namespace) = fixture::<2>();
    let r = registration();
    let parent = acquire(&mut runtime, r, scope(), 1);
    let shared = namespace.shared();
    shared.lock().rename("/jail/cwd", "/jail/moved").unwrap();
    shared.lock().mkdir("/jail/cwd", 1, 1, 0o755).unwrap();
    runtime
        .derive_caller(r, scope(), child(11), || true)
        .unwrap();
    let derived = acquire(&mut runtime, r, child(11), 2);
    assert_eq!(runtime.metadata(r, derived), runtime.metadata(r, parent));
    assert_eq!(
        runtime.policy_mut(r).unwrap().cwd(child(11)).unwrap(),
        "/moved"
    );
    // Credentials come from each dispatch, never from the parent binding.
    assert_eq!(stat_reply(&runtime, derived, "next/.", 43, 43).0, 13);
    runtime
        .change_root(r, scope(), next(12), "next", OWNER, || true)
        .unwrap();
    let narrowed = acquire(&mut runtime, r, next(12), 3);
    assert_ne!(runtime.metadata(r, narrowed), runtime.metadata(r, derived));
    runtime.retire_caller(scope().caller);
    assert_eq!(stat_reply(&runtime, derived, "next", 42, 42).0, 0);
    release(&mut runtime, parent);
    release(&mut runtime, narrowed);
    assert_eq!(shared.lock().open_handle_count(), 2);
    runtime.retire_caller(child(11).caller);
    release(&mut runtime, derived);
    assert_eq!(shared.lock().open_handle_count(), 0);
}

#[test]
fn derived_caller_revalidation_capacity_and_replay_fail_closed() {
    let (mut runtime, namespace) = fixture::<2>();
    let r = registration();
    let wrong = Registration {
        epoch: r.epoch + 1,
        ..r
    };
    assert_eq!(
        runtime.derive_caller(wrong, scope(), child(11), || panic!()),
        Err(Error::Identity)
    );
    assert_eq!(
        runtime.derive_caller(r, scope(), next(11), || panic!()),
        Err(Error::Scope)
    );
    assert_eq!(
        runtime.derive_caller(
            r,
            scope(),
            Scope {
                namespace_id: 100,
                ..child(11)
            },
            || panic!()
        ),
        Err(Error::Scope)
    );
    assert_eq!(
        runtime.derive_caller(r, next(11), child(12), || panic!()),
        Err(Error::Scope)
    );
    assert_eq!(
        runtime.derive_caller(r, scope(), child(11), || false),
        Err(Error::Identity)
    );
    assert_eq!(
        runtime.derive_caller(r, scope(), child(11), || true),
        Err(Error::Generation)
    );
    let parent = acquire(&mut runtime, r, scope(), 1);
    assert!(matches!(
        runtime.request(r, &retain(r, child(11), 2)).unwrap(),
        wire::Frame::Refused { .. }
    ));
    runtime
        .derive_caller(r, scope(), child(12), || true)
        .unwrap();
    assert_eq!(
        runtime.derive_caller(r, scope(), child(13), || panic!()),
        Err(Error::Scope)
    );
    let another = Scope {
        caller: Caller {
            pid: 30,
            ..child(12).caller
        },
        ..child(13)
    };
    assert_eq!(
        runtime.derive_caller(r, scope(), another, || panic!()),
        Err(Error::Capacity)
    );
    runtime.retire_caller(child(12).caller);
    runtime.derive_caller(r, scope(), another, || true).unwrap();
    runtime.retire_registration(r).unwrap();
    assert_eq!(
        runtime.derive_caller(r, scope(), child(14), || panic!()),
        Err(Error::Identity)
    );
    release(&mut runtime, parent);
    assert_eq!(namespace.shared().lock().open_handle_count(), 0);
}

#[test]
fn retained_handoff_outlives_source_policy_but_never_its_exact_lease() {
    let (mut runtime, namespace) = fixture::<2>();
    let r = registration();
    let parent = acquire(&mut runtime, r, scope(), 1);
    let expected = runtime.metadata(r, parent).unwrap();
    runtime.retire_caller(scope().caller);
    runtime
        .policy_mut(r)
        .unwrap()
        .derive_retained_caller(parent, child(11), || true)
        .unwrap();
    let derived = acquire(&mut runtime, r, child(11), 2);
    assert_eq!(runtime.metadata(r, derived).unwrap(), expected);
    release(&mut runtime, parent);
    assert_eq!(
        runtime
            .policy_mut(r)
            .unwrap()
            .derive_retained_caller(parent, child(12), || panic!()),
        Err(Error::Scope)
    );
    runtime.retire_caller(child(11).caller);
    release(&mut runtime, derived);
    assert_eq!(namespace.shared().lock().open_handle_count(), 0);
}

#[test]
fn retained_handoff_revalidation_replay_and_namespace_checks() {
    let (mut runtime, _) = fixture::<2>();
    let r = registration();
    let parent = acquire(&mut runtime, r, scope(), 1);
    assert_eq!(
        runtime.policy_mut(r).unwrap().derive_retained_caller(
            parent,
            Scope {
                namespace_id: 999,
                ..child(11)
            },
            || panic!()
        ),
        Err(Error::Scope)
    );
    assert_eq!(
        runtime
            .policy_mut(r)
            .unwrap()
            .derive_retained_caller(parent, child(11), || false),
        Err(Error::Identity)
    );
    assert_eq!(
        runtime
            .policy_mut(r)
            .unwrap()
            .derive_retained_caller(parent, child(11), || panic!()),
        Err(Error::Generation)
    );
    runtime
        .policy_mut(r)
        .unwrap()
        .derive_retained_caller(parent, child(12), || true)
        .unwrap();
    assert_eq!(
        runtime.policy_mut(r).unwrap().derive_retained_caller(
            parent,
            Scope {
                caller: Caller {
                    pid: 30,
                    ..child(13).caller
                },
                ..child(13)
            },
            || panic!()
        ),
        Err(Error::Capacity)
    );
    runtime.retire_registration(r).unwrap();
    assert_eq!(
        runtime
            .policy_mut(r)
            .unwrap()
            .derive_retained_caller(parent, child(14), || panic!()),
        Err(Error::Retired)
    );
}
