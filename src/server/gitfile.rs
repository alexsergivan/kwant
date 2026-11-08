//! Reading small files under a `.git` that a bot can write (missions' `git_head`, Dev mode's
//! `head_branch`). Everything there is untrusted: never follow a symlink, never block on a FIFO,
//! socket or device, never read a hard link (it may be a host file's other name), and cap sizes.
//! On Mac and Linux every read walks from an open folder with `openat`
//! ([`crate::sandbox::safe_fs::Dir`], S-8), so a folder the bot swaps for a symlink between two
//! checks is never followed. On Windows the same refusals hold for every reparse point (symlink,
//! junction, mount point, cloud placeholder), DOS device names (`CON`, `NUL`, `COM1`...) and
//! alternate data streams; a file that can't be checked that way is `None`, never read some
//! weaker way.

use crate::sandbox::safe_fs::Dir;
use std::io::Read;
#[cfg(unix)]
use std::path::Path;

/// HEAD, a loose ref or a `.git` file is tiny; a bigger one isn't git's.
pub(crate) const REF_MAX: u64 = 4 * 1024;
/// packed-refs can list many refs, but never this many for a bot's folder.
pub(crate) const PACKED_MAX: u64 = 1024 * 1024;

/// Reads `rel` (`/`-separated plain names) below the open folder `dir`: every component must be a
/// real folder and the last a regular file of at most `max` bytes with one link. A symlink is
/// never followed and a FIFO, socket or device never blocks it.
pub(crate) fn read(dir: &Dir, rel: &str, max: u64) -> Option<String> {
    let f = open(dir, rel)?;
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
    #[cfg(windows)]
    if !windows_one_plain_file(&f) {
        return None;
    }
    let mut buf = Vec::with_capacity(m.len() as usize);
    f.take(max + 1).read_to_end(&mut buf).ok()?;
    if buf.len() as u64 > max {
        return None;
    }
    String::from_utf8(buf).ok()
}

/// Mac and Linux: [`Dir::open_file`] walks with `O_NOFOLLOW|O_DIRECTORY` from `dir`'s descriptor
/// and opens the last part `O_NOFOLLOW|O_NONBLOCK`, so nothing is resolved twice.
#[cfg(unix)]
fn open(dir: &Dir, rel: &str) -> Option<std::fs::File> {
    dir.open_file(Path::new(rel)).ok()
}

/// Elsewhere (Windows) [`Dir`] is a checked path: `dir` itself and every part below it must be a
/// real folder (no reparse point of any kind) and the last a plain file, then it is opened as
/// itself, never as a link's target, and [`windows_one_plain_file`] checks the handle.
#[cfg(not(unix))]
fn open(dir: &Dir, rel: &str) -> Option<std::fs::File> {
    let mut at = dir.path().to_path_buf();
    if !plain_dir(&std::fs::symlink_metadata(&at).ok()?) {
        return None;
    }
    let parts: Vec<&str> = rel.split('/').collect();
    for (i, p) in parts.iter().enumerate() {
        #[cfg(windows)]
        if !windows_plain_name(p) {
            return None;
        }
        if p.is_empty() || *p == "." || *p == ".." {
            return None;
        }
        at.push(p);
        let m = std::fs::symlink_metadata(&at).ok()?;
        let last = i + 1 == parts.len();
        if (last && !plain_file(&m)) || (!last && !plain_dir(&m)) {
            return None;
        }
    }
    let mut o = std::fs::OpenOptions::new();
    o.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // A symlink or junction swapped in after the check opens as itself, not its target, and
        // the handle check refuses it
        o.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    o.open(&at).ok()
}

/// `m` (from `symlink_metadata`) is a real folder: not a symlink and, on Windows, no reparse point
/// of any kind. Mac and Linux need none of this: [`Dir`] walks with `O_NOFOLLOW`.
#[cfg(not(unix))]
fn plain_dir(m: &std::fs::Metadata) -> bool {
    m.file_type().is_dir() && !reparse_point(m)
}

/// `m` (from `symlink_metadata`) is a regular file, never a link or (on Windows) a reparse point.
#[cfg(not(unix))]
fn plain_file(m: &std::fs::Metadata) -> bool {
    m.file_type().is_file() && !reparse_point(m)
}

/// Any reparse point: std only calls the name-surrogate ones (symlinks, junctions) links, and a
/// placeholder or app link would otherwise pass as a plain file or folder.
#[cfg(windows)]
fn reparse_point(m: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    m.file_attributes() & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(all(not(unix), not(windows)))]
fn reparse_point(_: &std::fs::Metadata) -> bool {
    false
}

/// A path component Windows takes as the file it names: no `:` (a drive or a data stream), no
/// `\`, no trailing `.` or space (dropped by Win32), and not a DOS device (`CON`, `NUL.txt`...).
#[cfg(any(windows, test))]
fn windows_plain_name(p: &str) -> bool {
    const DEVICES: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    let stem = p.split('.').next().unwrap_or("").trim_end_matches(' ').to_ascii_uppercase();
    let numbered = |pre: &str| {
        stem.strip_prefix(pre).is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "\u{b9}" | "\u{b2}" | "\u{b3}"))
    };
    !p.is_empty()
        && !p.contains([':', '\\'])
        && !p.ends_with(['.', ' '])
        && !DEVICES.contains(&stem.as_str())
        && !numbered("COM")
        && !numbered("LPT")
}

/// The open handle is a file on disk (not a console, pipe or device), not a reparse point or a
/// folder, with exactly one name (git never hard-links these). Anything unreadable is `false`.
#[cfg(windows)]
fn windows_one_plain_file(f: &std::fs::File) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, GetFileType, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_TYPE_DISK,
    };
    let h = f.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
    // SAFETY: `h` is the live handle `f` owns for this whole call; `info` is a plain out-struct
    // that GetFileInformationByHandle fills.
    unsafe {
        if GetFileType(h) != FILE_TYPE_DISK {
            return false;
        }
        let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
        if GetFileInformationByHandle(h, &mut info) == 0 {
            return false;
        }
        info.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY) == 0 && info.nNumberOfLinks == 1
    }
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

#[cfg(test)]
mod name_tests {
    #[test]
    fn windows_names_refuse_devices_streams_and_trailing_dots() {
        use super::windows_plain_name as ok;
        for good in ["HEAD", "packed-refs", "main", "feature", "console", "CONTROL", "COM10", "a.b", ".git"] {
            assert!(ok(good), "{good}");
        }
        for bad in ["", "CON", "con", "NUL.txt", "aux.git", "COM1", "lpt9", "COM\u{b9}", "PRN ", "HEAD:x", "a\\b", "HEAD.", "HEAD "] {
            assert!(!ok(bad), "{bad}");
        }
    }
}

/// The Windows refusals, with real junctions, symlinks and hard links on NTFS.
#[cfg(all(test, windows))]
pub(crate) mod windows_tests {
    use super::*;
    use std::path::Path;

    fn read(base: &Path, rel: &str, max: u64) -> Option<String> {
        super::read(&Dir::open(base).unwrap(), rel, max)
    }

    /// A junction needs no privilege, unlike a symlink. cmd reads a `/` in
    /// a path as a switch, so both paths go over with backslashes only.
    pub(crate) fn junction(link: &Path, target: &Path) {
        let back = |p: &Path| p.to_string_lossy().replace('/', "\\");
        let out = std::process::Command::new("cmd").arg("/C").arg("mklink").arg("/J").arg(back(link)).arg(back(target)).output().unwrap();
        assert!(out.status.success(), "mklink /J: {}", String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    fn reads_one_regular_file_and_refuses_the_rest_on_windows() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("a")).unwrap();
        std::fs::write(d.path().join("a/f"), "hi").unwrap();
        assert_eq!(read(d.path(), "a/f", 10).as_deref(), Some("hi"));
        assert_eq!(read(d.path(), "a/f", 1), None, "over the cap");
        assert_eq!(read(d.path(), "a", 10), None, "a folder");
        assert_eq!(read(d.path(), "a/f:alt", 10), None, "a data stream");
        assert_eq!(read(d.path(), "a/f.", 10), None, "a trailing dot Win32 would drop");
        assert_eq!(read(d.path(), "a/NUL", 10), None, "a device");
        assert_eq!(read(d.path(), "CON", 10), None, "the console");

        // a junction anywhere on the way
        let elsewhere = d.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("f"), "LEAKED").unwrap();
        junction(&d.path().join("j"), &elsewhere);
        assert_eq!(read(d.path(), "j/f", 10), None, "through a junction");

        // a second name for the same file
        std::fs::hard_link(d.path().join("a/f"), d.path().join("g")).unwrap();
        assert_eq!(read(d.path(), "a/f", 10), None, "a file with two names");
        assert_eq!(read(d.path(), "g", 10), None);

        // a file symlink (the runner is elevated; on a PC without the right it can't be made,
        // and then there's nothing to refuse)
        match std::os::windows::fs::symlink_file(elsewhere.join("f"), d.path().join("s")) {
            Ok(()) => assert_eq!(read(d.path(), "s", 10), None, "a file symlink"),
            Err(e) => eprintln!("no symlink here ({e}); the junction case covers links"),
        }
    }
}
