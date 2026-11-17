use super::*;
use sunlight_fs::{Credential, FsError, Vfs};

const OWNER: Credential = Credential { uid: 42, gid: 42 };

fn fixture() -> (Runtime<1, 8, 1>, DirectoryNamespace) {
    let mut fs = Vfs::new();
    fs.mount_tmpfs("/", Tmpfs::new(&[])).unwrap();
    fs.mkdir("/jail", 42, 42, 0o755).unwrap();
    fs.mkdir("/jail/cwd", 42, 42, 0o755).unwrap();
    fs.mkdir("/jail/cwd/next", 42, 42, 0o700).unwrap();
    fs.mkdir("/jail/mount", 42, 42, 0o755).unwrap();
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

#[test]
fn transition_follows_renamed_cwd_preserves_root_and_old_lease_until_release() {
    let (mut runtime, namespace) = fixture();
    let r = registration();
    let old = acquire(&mut runtime, r, scope(), 1);
    let before = runtime.metadata(r, old).unwrap();
    let destination = stat_reply(&runtime, old, "next", 42, 42);
    assert_eq!(destination.0, 0);
    let fs = namespace.shared();
    fs.lock().rename("/jail/cwd", "/jail/moved").unwrap();
    fs.lock().mkdir("/jail/cwd", 7, 7, 0o755).unwrap();
    runtime
        .change_directory(r, scope(), next(11), "next", OWNER, || true)
        .unwrap();
    assert_eq!(runtime.metadata(r, old).unwrap(), before);
    let new = acquire(&mut runtime, r, next(11), 2);
    let after = runtime.metadata(r, new).unwrap();
    assert_eq!(stat_reply(&runtime, old, ".", 42, 42).0, 5);
    assert_eq!(stat_reply(&runtime, new, ".", 42, 42), destination);
    assert_eq!(after[0], before[0]);
    assert_ne!(after[1].0, before[1].0);
    assert_eq!(after[1].1.mode & 0o777, 0o700);
    assert!(matches!(
        runtime.request(r, &retain(r, scope(), 3)).unwrap(),
        wire::Frame::Refused { .. }
    ));
    assert_eq!(runtime.withdraw(r, scope()), Err(Error::Scope));
    assert_eq!(fs.lock().open_handle_count(), 4);
    runtime
        .request(r, &wire::Frame::Release(old).encode().unwrap())
        .unwrap();
    assert_eq!(fs.lock().open_handle_count(), 2);
    // Cross a mount using an absolute path rooted at the retained jail.
    runtime
        .change_directory(r, next(11), next(12), "/mount", OWNER, || true)
        .unwrap();
    let mounted = acquire(&mut runtime, r, next(12), 4);
    assert_eq!(runtime.metadata(r, mounted).unwrap()[0], before[0]);
    runtime.retire_caller(scope().caller);
    for receipt in [new, mounted] {
        runtime
            .request(r, &wire::Frame::Release(receipt).encode().unwrap())
            .unwrap();
    }
    assert_eq!(fs.lock().open_handle_count(), 0);
}

#[test]
fn failed_lookup_preserves_admission_and_consumes_generation() {
    let (mut runtime, namespace) = fixture();
    let r = registration();
    assert_eq!(
        runtime.change_directory(r, scope(), next(11), "missing", OWNER, || panic!(
            "lookup failed"
        )),
        Err(Error::Filesystem(FsError::NotFound))
    );
    assert_eq!(
        runtime.change_directory(r, scope(), next(11), "next", OWNER, || true),
        Err(Error::Generation)
    );
    let other = Credential { uid: 43, gid: 43 };
    assert_eq!(
        runtime.change_directory(r, scope(), next(12), "next", other, || true),
        Err(Error::Filesystem(FsError::PermissionDenied))
    );
    acquire(&mut runtime, r, scope(), 1);
    assert_eq!(namespace.shared().lock().open_handle_count(), 2);
    runtime
        .change_directory(r, scope(), next(13), "next", OWNER, || true)
        .unwrap();
    acquire(&mut runtime, r, next(13), 2);
}

#[test]
fn failed_revalidation_withdraws_admission_and_drops_provisional_pins() {
    let (mut runtime, namespace) = fixture();
    let r = registration();
    let receipt = acquire(&mut runtime, r, scope(), 1);
    let before = runtime.metadata(r, receipt).unwrap();
    assert_eq!(
        runtime.change_directory(r, scope(), next(11), "next", OWNER, || false),
        Err(Error::Identity)
    );
    assert_eq!(namespace.shared().lock().open_handle_count(), 2);
    assert_eq!(runtime.metadata(r, receipt).unwrap(), before);
    for (id, s) in [(2, scope()), (3, next(11))] {
        assert!(matches!(
            runtime.request(r, &retain(r, s, id)).unwrap(),
            wire::Frame::Refused { .. }
        ));
    }
    runtime
        .request(r, &wire::Frame::Release(receipt).encode().unwrap())
        .unwrap();
    assert_eq!(namespace.shared().lock().open_handle_count(), 0);
}

#[test]
fn stale_identity_scope_generation_and_retirement_cannot_change_policy() {
    let (mut runtime, namespace) = fixture();
    let r = registration();
    let wrong = Registration {
        epoch: r.epoch + 1,
        ..r
    };
    assert_eq!(
        runtime.change_directory(wrong, scope(), next(11), "next", OWNER, || true),
        Err(Error::Identity)
    );
    for field in 0..5 {
        let mut s = next(11);
        match field {
            0 => s.caller.pid += 1,
            1 => s.caller.incarnation += 1,
            2 => s.caller.address_space_generation += 1,
            3 => s.namespace_id += 1,
            _ => s.namespace_generation = scope().namespace_generation,
        }
        assert!(runtime
            .change_directory(r, scope(), s, "next", OWNER, || true)
            .is_err());
    }
    runtime
        .change_directory(r, scope(), next(11), "next", OWNER, || true)
        .unwrap();
    assert_eq!(
        runtime.change_directory(r, scope(), next(12), ".", OWNER, || true),
        Err(Error::Scope)
    );
    runtime.retire_registration(r).unwrap();
    assert_eq!(
        runtime.change_directory(r, next(11), next(12), ".", OWNER, || true),
        Err(Error::Identity)
    );
    assert_eq!(namespace.shared().lock().open_handle_count(), 0);
}
