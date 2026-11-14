from pathlib import Path
p=Path('sunlight-libc/src/c_file.rs');s=p.read_text();s+='''
/// Native realpath: caller-owned output must have PATH_MAX (256) bytes.
/// NULL output allocates exactly the result plus NUL through libc malloc.
/// Canonical output is a raced observation, never an authorization token.
#[no_mangle]
pub unsafe extern "C" fn realpath(path: *const u8, output: *mut u8) -> *mut u8 {
    if path.is_null() {
        errno::set_errno(errno::EINVAL);
        return core::ptr::null_mut();
    }
    let mut len = 0;
    while len < crate::MAX_PATH && *path.add(len) != 0 { len += 1; }
    if len == crate::MAX_PATH {
        errno::set_errno(36); // ENAMETOOLONG
        return core::ptr::null_mut();
    }
    if len == 0 {
        errno::set_errno(errno::ENOENT);
        return core::ptr::null_mut();
    }
    let mut canonical = [0u8; crate::MAX_PATH];
    let n = match crate::resolve_path(core::slice::from_raw_parts(path, len), &mut canonical) {
        Ok(n) => n,
        Err(sys::Errno::TooBig) => { errno::set_errno(36); return core::ptr::null_mut(); }
        Err(e) => { errno::set_from_errno(e); return core::ptr::null_mut(); }
    };
    let destination = if output.is_null() { crate::alloc::malloc(n + 1) } else { output };
    if destination.is_null() { return destination; }
    core::ptr::copy_nonoverlapping(canonical.as_ptr(), destination, n + 1);
    destination
}
''';p.write_text(s)
p=Path('sunlight-libc/include/stdlib.h');s=p.read_text().replace('char *getenv(const char *name);','''char *getenv(const char *name);
/* Existing-object canonical query through native VFS. NULL output uses malloc;
 * caller-owned output requires PATH_MAX bytes (256), including NUL. A returned
 * path is not an atomic authorization token for a later open. */
char *realpath(const char *path, char *output);''');p.write_text(s)
p=Path('sunlight-libc/include/limits.h');s=p.read_text().replace('#define SSIZE_MAX LONG_MAX','#define SSIZE_MAX LONG_MAX\n#define PATH_MAX 256');p.write_text(s)
p=Path('sunlight-libc/include/errno.h');s=p.read_text().replace('#define ENOSYS 38','#define ENAMETOOLONG 36\n#define ENOSYS 38');p.write_text(s)
p=Path('sunshell/src/main.rs');s=p.read_text();start=s.index('            let Some(next) = normalize_path(&self.cwd, target)',s.index('fn cmd_cd('));end=s.index('\n            self.env.set("OLDPWD"',start);s=s[:start]+'''            let Some(next) = resolve_directory(&self.cwd, target) else {
                return self.builtin_failure(b"cd: directory unavailable or access denied\\n");
            };
            if sunlight_libc::chdir(next.as_bytes()).is_err() {
                return self.builtin_failure(b"cd: directory unavailable or access denied\\n");
            }
''' +s[end:];idx=s.index('    fn normalize_path(');s=s[:idx]+'''    // Preserve components and whitespace until the authoritative VFS walk.
    fn resolve_directory(cwd: &str, target: &str) -> Option<alloc::string::String> {
        if target.is_empty() { return None; }
        let input = if target.starts_with('/') { alloc::string::String::from(target) }
                    else { alloc::format!("{}/{}", cwd.trim_end_matches('/'), target) };
        let mut output = [0u8; sunlight_libc::MAX_PATH];
        let n = sunlight_libc::resolve_path(input.as_bytes(), &mut output).ok()?;
        let canonical = core::str::from_utf8(&output[..n]).ok()?;
        if !stat_is_dir(canonical) { return None; }
        Some(alloc::string::String::from(canonical))
    }

''' +s[idx:];p.write_text(s)
p=Path('sunshell/src/native_script.rs');s=p.read_text();start=s.index('        let name = path(name, context)?;',s.index('    fn directory('));end=s.index('\n    fn source(',start);s=s[:start]+'''        resolve_directory(&context.cwd, name)
            .ok_or_else(|| Error::new(1, "cd: directory unavailable or access denied"))
    }
''' +s[end:];p.write_text(s)
p=Path('sunlight-files/src/main.rs');s=p.read_text();start=s.index('        let bytes = text.as_bytes();',s.index('    fn set(&mut self, text: &str)'));end=s.index('\n    fn ',start);s=s[:start]+'''        // Preserve the user's path until VFS checks existence and traversal.
        // In particular, whitespace and components before '..' are meaningful.
        let text = if text.is_empty() { "/" } else { text };
        if text.len() >= PATH_LEN || text.as_bytes().contains(&0) { return false; }
        self.buf[..text.len()].copy_from_slice(text.as_bytes());
        self.len = text.len();
        true
    }
''' +s[end:];idx=s.index('        debug_log("[FILES] navigate_start',s.index('    fn navigate_to('));s=s[:idx]+'''        let input = if target.as_str().starts_with('/') {
            alloc::string::String::from(target.as_str())
        } else {
            alloc::format!("{}/{}", self.current_path.as_str().trim_end_matches('/'), target.as_str())
        };
        let mut output = [0u8; PATH_LEN];
        let target = match libc::resolve_path(input.as_bytes(), &mut output)
            .ok().and_then(|n| core::str::from_utf8(&output[..n]).ok())
            .and_then(PathBuf::from_str) {
            Some(path) => path,
            None => { self.set_error("Folder unavailable or access denied"); return false; }
        };
''' +s[idx:];s=s.replace('fn thumbnail_paths_are_lexically_normalized()', 'fn path_storage_preserves_components_until_vfs_lookup()').replace('            "/pictures/image.png"\n        );','            "//pictures/./sub/../image.png"\n        );').replace('            "/pictures"\n        );\n    }\n    #[test]', '            "/../../pictures"\n        );\n        assert_eq!(PathBuf::from_str("  name  ").unwrap().as_str(), "  name  ");\n        assert!(PathBuf::from_str("/bad\\0name").is_none());\n    }\n    #[test]');p.write_text(s)
