extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Algorithm, Argon2, Params, Version,
};
use sunlight_fs::{lookup_by_name, parse_passwd, parse_shadow};
use sunlight_ipc::{ipc_call, nameserver_lookup, shm_create, shm_free, IpcMsg};
use zeroize::Zeroize;

pub const MAX_USERNAME_LEN: usize = 64;
pub const MAX_PASSWORD_LEN: usize = 128;
pub const AUTH_PASSWORD_OP: u64 = 6;
pub const AUTH_SUCCESS: u64 = 1;
pub const AUTH_FAILURE: u64 = 0xff;
pub const AUTH_SHM_SIZE: usize = 4096;
pub const AUTH_PASSWD_PATH: &str = "/etc/passwd";
pub const AUTH_SHADOW_PATH: &str = "/etc/shadow";

const ARGON2_MEMORY_KIB: u32 = 64;
const ARGON2_ITERATIONS: u32 = 3;
const ARGON2_LANES: u32 = 1;
const ARGON2_OUTPUT_LEN: usize = 32;
const ARGON2_PREFIX: &str = "$argon2id$";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthError {
    InvalidInput,
    Unavailable,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthSuccess {
    pub uid: u32,
    pub gid: u32,
}

pub fn authenticate_password(username: &[u8], password: &[u8]) -> Option<AuthSuccess> {
    if !username_valid(username) || !password_valid(password) {
        return None;
    }

    let uac = nameserver_lookup("uac")?;
    let (ptr, token) = shm_create(AUTH_SHM_SIZE, 0).ok()?;
    unsafe {
        core::ptr::copy_nonoverlapping(password.as_ptr(), ptr, password.len());
        *ptr.add(password.len()) = 0;
    }

    let mut msg = IpcMsg::with_label(AUTH_PASSWORD_OP).with_cap(0, token);
    pack_nul_terminated(&mut msg, 0, username);
    let reply = ipc_call(uac, msg);

    unsafe {
        core::ptr::write_bytes(ptr, 0, password.len().saturating_add(1));
    }
    let _ = shm_free(token);

    if reply.label == AUTH_SUCCESS {
        Some(AuthSuccess {
            uid: reply.words[0] as u32,
            gid: reply.words[1] as u32,
        })
    } else {
        None
    }
}

pub fn hash_password(password: &[u8]) -> Result<String, AuthError> {
    if !password_valid(password) {
        return Err(AuthError::InvalidInput);
    }

    let argon2 = password_hasher()?;
    let mut salt_bytes = [0u8; 16];
    fill_salt_bytes(&mut salt_bytes)?;
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|_| AuthError::Unavailable)?;
    salt_bytes.zeroize();

    argon2
        .hash_password(password, &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| AuthError::Unavailable)
}

pub fn verify_shadow_credentials(
    passwd_data: &[u8],
    shadow_data: &[u8],
    username: &[u8],
    password: &[u8],
) -> Result<AuthSuccess, AuthError> {
    if !username_valid(username) || !password_valid(password) {
        return Err(AuthError::InvalidInput);
    }

    let (passwd_entries, passwd_count) = parse_passwd(passwd_data);
    let Some(entry) = lookup_by_name(&passwd_entries[..passwd_count], username) else {
        return Err(AuthError::Failed);
    };

    let (shadow_entries, shadow_count) = parse_shadow(shadow_data);
    for shadow_entry in &shadow_entries[..shadow_count] {
        let uname_len = nul_len(&shadow_entry.username);
        if uname_len != username.len() || &shadow_entry.username[..uname_len] != username {
            continue;
        }

        let hash_len = nul_len(&shadow_entry.password);
        let hash_str =
            core::str::from_utf8(&shadow_entry.password[..hash_len]).map_err(|_| AuthError::Failed)?;
        verify_password_hash(hash_str, password)?;
        return Ok(AuthSuccess {
            uid: entry.uid,
            gid: entry.gid,
        });
    }

    Err(AuthError::Failed)
}

pub fn migrate_shadow_contents(passwd_data: &[u8], shadow_data: &[u8]) -> Result<String, AuthError> {
    let shadow_str = core::str::from_utf8(shadow_data).map_err(|_| AuthError::Failed)?;
    let (passwd_entries, passwd_count) = parse_passwd(passwd_data);
    let mut migrated = String::new();
    let mut seen = [false; 16];

    for line in shadow_str.lines() {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(3, ':');
        let Some(username) = parts.next() else {
            return Err(AuthError::Failed);
        };
        let Some(secret) = parts.next() else {
            return Err(AuthError::Failed);
        };
        let rest = parts.next().unwrap_or("");

        if let Some(index) = passwd_index(&passwd_entries[..passwd_count], username.as_bytes()) {
            seen[index] = true;
        }

        if secret.starts_with(ARGON2_PREFIX) || secret == "!" || secret == "*" {
            migrated.push_str(line);
        } else {
            let hashed = hash_password(secret.as_bytes())?;
            if rest.is_empty() {
                migrated.push_str(&format!("{username}:{hashed}"));
            } else {
                migrated.push_str(&format!("{username}:{hashed}:{rest}"));
            }
        }
        migrated.push('\n');
    }

    for (index, entry) in passwd_entries[..passwd_count].iter().enumerate() {
        if seen[index] {
            continue;
        }
        let username_len = nul_len(&entry.username);
        let username =
            core::str::from_utf8(&entry.username[..username_len]).map_err(|_| AuthError::Failed)?;
        let generated = match username {
            "root" => {
                let hash = hash_password(b"root")?;
                format!("{username}:{hash}:0:0:99999:7:::")
            }
            "user" => {
                let hash = hash_password(b"user")?;
                format!("{username}:{hash}:0:0:99999:7:::")
            }
            _ => format!("{username}:!:0:0:99999:7:::"),
        };
        migrated.push_str(&generated);
        migrated.push('\n');
    }

    Ok(migrated)
}

fn password_hasher() -> Result<Argon2<'static>, AuthError> {
    let params = Params::new(
        ARGON2_MEMORY_KIB,
        ARGON2_ITERATIONS,
        ARGON2_LANES,
        Some(ARGON2_OUTPUT_LEN),
    )
    .map_err(|_| AuthError::Unavailable)?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

fn verify_password_hash(stored: &str, password: &[u8]) -> Result<(), AuthError> {
    if !stored.starts_with(ARGON2_PREFIX) {
        return Err(AuthError::Failed);
    }
    let parsed = PasswordHash::new(stored).map_err(|_| AuthError::Failed)?;
    password_hasher()?
        .verify_password(password, &parsed)
        .map_err(|_| AuthError::Failed)
}

fn fill_salt_bytes(bytes: &mut [u8]) -> Result<(), AuthError> {
    #[cfg(test)]
    {
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = (index as u8).wrapping_mul(19).wrapping_add(7);
        }
        Ok(())
    }

    #[cfg(not(test))]
    {
        if sunlight_libc::getrandom(bytes, 0) == bytes.len() as isize {
            Ok(())
        } else {
            Err(AuthError::Unavailable)
        }
    }
}

fn passwd_index(entries: &[sunlight_fs::PasswdEntry], username: &[u8]) -> Option<usize> {
    entries.iter().position(|entry| {
        let len = nul_len(&entry.username);
        len == username.len() && &entry.username[..len] == username
    })
}

fn nul_len(bytes: &[u8]) -> usize {
    bytes.iter().position(|&byte| byte == 0).unwrap_or(bytes.len())
}

fn username_valid(username: &[u8]) -> bool {
    !username.is_empty()
        && username.len() <= MAX_USERNAME_LEN
        && username
            .iter()
            .all(|byte| *byte != 0 && *byte != b':' && *byte != b'\n' && *byte != b'\r')
}

fn password_valid(password: &[u8]) -> bool {
    !password.is_empty()
        && password.len() <= MAX_PASSWORD_LEN
        && password.iter().all(|byte| *byte != 0 && *byte != b'\n' && *byte != b'\r')
}

fn pack_nul_terminated(msg: &mut IpcMsg, start: usize, bytes: &[u8]) {
    let mut offset = 0usize;
    for word_index in start..msg.words.len() {
        let mut word = 0u64;
        for byte_index in 0..8 {
            if offset < bytes.len() {
                word |= (bytes[offset] as u64) << (byte_index * 8);
                offset += 1;
            }
        }
        msg.words[word_index] = word;
    }
    msg.word_count = msg.words.len() as u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD_DATA: &[u8] =
        b"root:x:0:0:root:/root:/bin/sh\nuser:x:1000:1000:Regular User:/home/user:/bin/sh\n";

    #[test]
    fn hashes_are_argon2id() {
        let hash = hash_password(b"root").unwrap();
        assert!(hash.starts_with(ARGON2_PREFIX));
    }

    #[test]
    fn verifies_hashed_passwords_and_rejects_bad_credentials() {
        let root_hash = hash_password(b"root").unwrap();
        let shadow = format!("root:{root_hash}:0:0:99999:7:::\n");

        let success =
            verify_shadow_credentials(PASSWD_DATA, shadow.as_bytes(), b"root", b"root").unwrap();
        assert_eq!(success.uid, 0);
        assert_eq!(success.gid, 0);
        assert!(verify_shadow_credentials(PASSWD_DATA, shadow.as_bytes(), b"root", b"bad").is_err());
        assert!(verify_shadow_credentials(PASSWD_DATA, shadow.as_bytes(), b"missing", b"root").is_err());
    }

    #[test]
    fn rejects_plaintext_shadow_entries() {
        assert!(
            verify_shadow_credentials(
                PASSWD_DATA,
                b"root:root:0:0:99999:7:::\n",
                b"root",
                b"root"
            )
            .is_err()
        );
    }

    #[test]
    fn migrates_plaintext_and_missing_development_entries() {
        let migrated = migrate_shadow_contents(PASSWD_DATA, b"root:root:0:0:99999:7:::\n").unwrap();
        assert!(migrated.contains("$argon2id$"));
        assert!(migrated.contains("user:$argon2id$"));
        assert!(!migrated.contains(":root:"));
        assert!(!migrated.contains(":user:"));
    }
}
