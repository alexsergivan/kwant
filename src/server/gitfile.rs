//! Reading small files under a `.git` that a bot can write (missions' `git_head`, Dev mode's
//! `head_branch`). Everything there is untrusted: never follow a symlink, never block on a FIFO,
//! socket or device, never read a hard link (it may be a host file's other name), and cap sizes.

use std::io::Read;
use std::path::Path;

/// HEAD, a loose ref or a `.git` file is tiny; a bigger one isn't git's.
pub(crate) const REF_MAX: u64 = 4 * 1024;
/// packed-refs can list many refs, but never this many for a bot's folder.
pub(crate) const PACKED_MAX: u64 = 1024 * 1024;

/// Reads `<base>/<rel>` (`rel` is `/`-separated and already checked) only when every component
/// below `base` is a real folder and the last a regular file of at most `max` bytes with one
/// link. Never follows a symlink and never blocks on a FIFO, socket or device.
pub(crate) fn read(base: &Path, rel: &str, max: u64) -> Option<String> {
    let mut at = base.to_path_buf();
    let parts: Vec<&str> = rel.split('/').collect();
    for (i, p) in parts.iter().enumerate() {
        at.push(p);
        let ft = std::fs::symlink_metadata(&at).ok()?.file_type();
        let last = i + 1 == parts.len();
        if (last && !ft.is_file()) || (!last && !ft.is_dir()) {
            return None;
        }
    }
    let mut o = std::fs::OpenOptions::new();
    o.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A symlink swapped in after the check fails to open; a FIFO opens without waiting
        o.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let f = o.open(&at).ok()?;
    let m = f.metadata().ok()?;
    if !m.file_type().is_file() || m.len() > max {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // git never hard-links these; a second name may be a host file's
        if m.nlink() > 1 {
            return None;
        }
    }
    let mut buf = Vec::with_capacity(m.len() as usize);
    f.take(max + 1).read_to_end(&mut buf).ok()?;
    if buf.len() as u64 > max {
        return None;
    }
    String::from_utf8(buf).ok()
}

/// A `ref:` target git would write: `refs/` then plain components of `[A-Za-z0-9._-]`, so it
/// can't leave `.git` (no `..`, no empty or absolute part, no backslash).
pub(crate) fn plain_ref(r: &str) -> bool {
    r.len() <= 1024
        && r.starts_with("refs/")
        && !r.contains("..")
        && r.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-'))
        && r.split('/').all(|c| !c.is_empty() && c != ".")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn reads_one_regular_file_and_nothing_else() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("a")).unwrap();
        std::fs::write(d.path().join("a/f"), "hi").unwrap();
        assert_eq!(read(d.path(), "a/f", 10).as_deref(), Some("hi"));
        assert_eq!(read(d.path(), "a/f", 1), None, "over the cap");
        assert_eq!(read(d.path(), "a", 10), None, "a folder");
        std::fs::hard_link(d.path().join("a/f"), d.path().join("g")).unwrap();
        assert_eq!(read(d.path(), "a/f", 10), None, "a file with two names");
        assert_eq!(read(d.path(), "g", 10), None);
    }
}
