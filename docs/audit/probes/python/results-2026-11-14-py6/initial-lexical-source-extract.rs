extern crate alloc;
const PATH_LEN:usize=256;
    fn normalize_path(cwd: &str, path: &str) -> Option<alloc::string::String> {
        let mut parts: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
        let full = if path.starts_with('/') {
            alloc::string::String::from(path)
        } else if cwd == "/" {
            alloc::format!("/{}", path)
        } else {
            alloc::format!("{}/{}", cwd, path)
        };

        for part in full.split('/') {
            if part.is_empty() || part == "." {
                continue;
            }
            if part == ".." {
                if !parts.is_empty() {
                    parts.pop();
                }
                continue;
            }
            if part.len() > 64 {
                return None;
            }
            parts.push(part);
        }

        if parts.is_empty() {
            return Some(alloc::string::String::from("/"));
        }
        let mut out = alloc::string::String::from("/");
        for (idx, part) in parts.iter().enumerate() {
            if idx > 0 {
                out.push('/');
            }
            out.push_str(part);
        }
        Some(out)
    }

#[derive(Clone, Copy)]
struct PathBuf {
    buf: [u8; PATH_LEN],
    len: usize,
}

impl PathBuf {
    const fn root() -> Self {
        let mut buf = [0u8; PATH_LEN];
        buf[0] = b'/';
        Self { buf, len: 1 }
    }

    fn from_str(text: &str) -> Option<Self> {
        let mut out = Self::root();
        if out.set(text) {
            Some(out)
        } else {
            None
        }
    }

    fn set(&mut self, text: &str) -> bool {
        // Preserve the user's path until VFS checks existence and traversal.
        // In particular, whitespace and components before '..' are meaningful.
        let text = if text.is_empty() { "/" } else { text };
        if text.len() >= PATH_LEN || text.as_bytes().contains(&0) { return false; }
        self.buf[..text.len()].copy_from_slice(text.as_bytes());
        self.len = text.len();
        true
    }

    fn join(&self, component: &str) -> Option<Self> {
        let mut out = *self;
        if out.len > 1 && out.buf[out.len - 1] != b'/' {
            if out.len >= PATH_LEN {
                return None;
            }
            out.buf[out.len] = b'/';
            out.len += 1;
        }
        if !out.set_suffix(component.as_bytes()) {
            return None;
        }
        Some(out)
    }

    fn set_suffix(&mut self, suffix: &[u8]) -> bool {
        if self.len == 0 {
            self.buf[0] = b'/';
            self.len = 1;
        }
        if self.len > 1 && self.buf[self.len - 1] != b'/' {
            if self.len >= PATH_LEN {
                return false;
            }
            self.buf[self.len] = b'/';
            self.len += 1;
        }
        for &b in suffix {
            if b == 0 || self.len >= PATH_LEN {
                return false;
            }
            self.buf[self.len] = b;
            self.len += 1;
        }
        true
    }

    fn parent(&self) -> Option<Self> {
        if self.len <= 1 {
            return None;
        }
        let mut end = self.len;
        while end > 1 && self.buf[end - 1] == b'/' {
            end -= 1;
        }
        while end > 1 && self.buf[end - 1] != b'/' {
            end -= 1;
        }
        if end <= 1 {
            Some(Self::root())
        } else {
            let mut out = *self;
            out.len = end - 1;
            if out.len == 0 {
                out.len = 1;
                out.buf[0] = b'/';
            }
            Some(out)
        }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("/")
    }
}
fn main(){
 for (cwd,input) in [("/tmp","."),("/tmp",".."),("/","/tmp//./../tmp"),("/tmp","missing/../x"),("/tmp","relative"),("/tmp"," /tmp ")]{
  let sh=normalize_path(cwd,input).unwrap();let fm=PathBuf::from_str(input).unwrap();
  println!("cwd={cwd:?} input={input:?} shell={sh:?} files={:?}",fm.as_str());
 }
 assert_eq!(normalize_path("/tmp","/tmp//./../tmp").unwrap(),"/tmp");
 assert_eq!(PathBuf::from_str("/tmp//./../tmp").unwrap().as_str(),"/tmp//./../tmp");
 assert_eq!(normalize_path("/tmp","relative").unwrap(),"/tmp/relative");
 assert_eq!(PathBuf::from_str("relative").unwrap().as_str(),"relative");
 assert_eq!(PathBuf::from_str("  filename  ").unwrap().as_str(),"  filename  ");
 assert!(PathBuf::from_str("/bad\0path").is_some());
 assert_eq!(PathBuf::from_str("/tmp").unwrap().parent().unwrap().as_str(),"/");
 assert_eq!(PathBuf::from_str("/tmp").unwrap().join("x").unwrap().as_str(),"/tmp/x");
}
