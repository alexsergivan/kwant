//! Reading small files under a `.git` that a bot can write (missions' `git_head`, Dev mode's
//! `head_branch`). Everything there is untrusted: never follow a symlink, never block on a FIFO,
//! socket or device, never read a hard link (it may be a host file's other name), and cap sizes.
//! Every read walks from an open folder with `openat` ([`crate::sandbox::safe_fs::Dir`], S-8), so
//! a folder the bot swaps for a symlink between two checks is never followed.

use crate::sandbox::safe_fs::Dir;
use std::io::Read;
use std::path::Path;

/// HEAD, a loose ref or a `.git` file is tiny; a bigger one isn't git's.
pub(crate) const REF_MAX: u64 = 4 * 1024;
/// packed-refs can list many refs, but never this many for a bot's folder.
pub(crate) const PACKED_MAX: u64 = 1024 * 1024;

/// Reads `rel` (`/`-separated plain names) below the open folder `dir` with [`Dir`]'s openat
/// walk (S-8): every component must be a real folder and the last a regular file of at most
/// `max` bytes with one link. Nothing is resolved twice, a symlink is never followed and a FIFO,
/// socket or device never blocks it.
pub(crate) fn read(dir: &Dir, rel: &str, max: u64) -> Option<String> {
    // `open_file` walks with O_NOFOLLOW|O_DIRECTORY, opens the last part O_NOFOLLOW|O_NONBLOCK
    // and checks with fstat that it holds a regular file
    let f = dir.open_file(Path::new(rel)).ok()?;
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

    fn read(base: &Path, rel: &str, max: u64) -> Option<String> {
        super::read(&Dir::open(base).unwrap(), rel, max)
    }

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

    #[test]
    fn never_follows_a_symlink_or_blocks_on_a_fifo() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(out.path().join("a")).unwrap();
        std::fs::write(out.path().join("a/f"), "secret").unwrap();
        symlink(out.path().join("a"), d.path().join("a")).unwrap();
        assert_eq!(read(d.path(), "a/f", 10), None, "a symlinked folder on the way");
        symlink(out.path().join("a/f"), d.path().join("f")).unwrap();
        assert_eq!(read(d.path(), "f", 10), None, "a symlinked file");
        assert_eq!(read(d.path(), "../x", 10), None, "`..` is no plain name");
        let fifo = std::ffi::CString::new(d.path().join("p").as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
        let (tx, rx) = std::sync::mpsc::channel();
        let base = d.path().to_path_buf();
        std::thread::spawn(move || tx.send(read(&base, "p", 10)).unwrap());
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)).expect("blocked on a FIFO"), None);
    }
}
