from pathlib import Path
p=Path('sunlight-fs/src/vfs/lookup.rs');s=p.read_text();a=s.index('        if input.is_empty()',s.index('pub fn resolve_existing('));s=s[:a]+'''        self.resolve_existing_authorized(cwd, input, cred, &mut |_| Ok(()))
    }

    /// Additional kernel capability restrictions are checked before each
    /// component metadata lookup, including components later removed by '..'.
    pub fn resolve_existing_authorized(
        &mut self, cwd: &str, input: &str, cred: &Credential,
        authorize: &mut dyn FnMut(&str) -> Result<(), FsError>,
    ) -> Result<ResolvedPath, FsError> {
'''+s[a:];s=s.replace('self.walk_components(&mut path, cwd, cred)?;','self.walk_components(&mut path, cwd, cred, authorize)?;').replace('self.walk_components(&mut path, input, cred)?;','self.walk_components(&mut path, input, cred, authorize)?;\n        authorize(&path)?;');s=s.replace('        cred: &Credential,\n    ) -> Result<(), FsError> {\n        for component','        cred: &Credential,\n        authorize: &mut dyn FnMut(&str) -> Result<(), FsError>,\n    ) -> Result<(), FsError> {\n        authorize(path)?;\n        for component');s=s.replace('            // Existence is checked','            authorize(path)?;\n            // Existence is checked');p.write_text(s)
for f in ['ipc/src/lib.rs','kernel/src/arch/x86_64/syscall.rs']:
 p=Path(f);s=p.read_text().replace('    NativeProcessIdentity = 178,','    NativeProcessIdentity = 178,\n    /// Existing canonical path: rdi input, rsi output, rdx capacity (with NUL).\n    ResolvePath = 179,');p.write_text(s)
p=Path('kernel/src/arch/x86_64/syscall.rs');s=p.read_text().replace('        178 => sys_native_process_identity(frame.rdi),','        178 => sys_native_process_identity(frame.rdi),\n        179 => sys_resolve_path(frame),');start=s.index('    // Resolve against current CWD if relative.',s.index('fn sys_chdir('));end=s.index('\n    crate::sched::with_scheduler(|s| {',start);s=s[:start]+'''    let resolved = match resolve_native_existing(path) {
        Ok(value) => value,
        Err(error) => return error,
    };
    if resolved.metadata.native.file_type != sunlight_fs::FileType::Directory {
        return ERR_ENOTDIR;
    }
    let abs_path = resolved.path.as_str();
''' +s[end:]
idx=s.index('fn sys_chdir(');s=s[:idx]+'''/// Snapshot authority before taking VFS; no scheduler/IPC lock inside lookup.
fn resolve_native_existing(path: &str) -> Result<sunlight_fs::vfs::ResolvedPath, u64> {
    let (uid, gid, _) = current_fs_actor();
    let cwd = crate::sched::with_scheduler(|s| s.current_shared_process().cwd.clone());
    let host_key_admin = has_host_key_admin();
    let trusted_kv = crate::sched::with_scheduler(|s| s.current_process().trusted_kv_service);
    let mut guard = crate::KERNEL_VFS.lock();
    let vfs = guard.as_mut().ok_or(ERR_EIO)?;
    vfs.resolve_existing_authorized(&cwd, path, &sunlight_fs::Credential { uid, gid }, &mut |name| {
        if (private_secret_child(name) && !host_key_admin)
            || (name == "/var/lib/sunlight-kv/broker.store" && !trusted_kv) {
            Err(sunlight_fs::FsError::PermissionDenied)
        } else { Ok(()) }
    }).map_err(fs_error_raw)
}

/// Read-only canonical observation. Capacity includes the terminating NUL;
/// short output buffers are untouched, and success returns payload length.
fn sys_resolve_path(frame: &mut SyscallFrame) -> u64 {
    if frame.rsi == 0 || frame.rdx == 0 { return ERR_EINVAL; }
    let bytes = match read_user_cstr(frame.rdi, USER_PATH_MAX) {
        Ok(bytes) => bytes,
        Err(crate::memory::user::UserMemoryError::StringTooLong) => return ERR_ERANGE,
        Err(error) => return user_memory_failure(error),
    };
    let path = match core::str::from_utf8(&bytes) { Ok(path) => path, Err(_) => return ERR_EINVAL };
    if path.is_empty() { return ERR_NOENT; }
    let resolved = match resolve_native_existing(path) { Ok(value) => value, Err(error) => return error };
    let len = resolved.path.len();
    if frame.rdx <= len as u64 { return ERR_ERANGE; }
    // Allocation was reserved by VFS; append NUL after dropping the VFS lock.
    let mut output = resolved.path.into_bytes();
    if output.try_reserve_exact(1).is_err() { return ERR_ENOMEM; }
    output.push(0);
    match copy_to_user(frame.rsi, &output) { Ok(()) => len as u64, Err(error) => error }
}

''' +s[idx:];p.write_text(s)
p=Path('sunlight-libc/src/sys.rs');s=p.read_text().replace('pub const SYS_STAT: u64 = 61;','pub const SYS_STAT: u64 = 61;\npub const SYS_RESOLVE_PATH: u64 = 179;');p.write_text(s)
p=Path('sunlight-libc/src/lib.rs');s=p.read_text();idx=s.index('/// Change the current process CWD');s=s[:idx]+'''/// Authoritative existing-object canonical path in the current namespace.
/// The result is an observation, not authority or a pin for a later operation.
/// Output capacity includes NUL; no partial output on a short buffer.
pub fn resolve_path(path: &[u8], output: &mut [u8]) -> Result<usize, Errno> {
    let mut input = [0u8; MAX_PATH];
    let ptr = cstr(&mut input, path)?;
    let ret = unsafe { sys::syscall3(sys::SYS_RESOLVE_PATH, ptr as u64, output.as_mut_ptr() as u64, output.len() as u64) };
    let len = sys::check(ret)? as usize;
    if len >= output.len() || output[len] != 0 { return Err(Errno::Failed); }
    Ok(len)
}

''' +s[idx:];p.write_text(s)
