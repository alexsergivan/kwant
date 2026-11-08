//! Reading a bot's folder from this computer while the bot may be changing it (test plan S-8).
//!
//! In vm mode the bot controls its folder from inside its container, so any folder or file in it
//! can turn into a symlink between a check and an open. A per-path `symlink_metadata` check and a
//! later `open` by path is a race the bot can win. [`Dir`] never resolves a path twice: it holds
//! the folder open and walks each component with `openat(…, O_NOFOLLOW|O_DIRECTORY)`, opens the
//! last one with `O_NOFOLLOW|O_NONBLOCK` and checks with `fstat` that what it holds is a regular
//! file. A component swapped for a symlink fails the open (`ELOOP` or `ENOTDIR`); nothing outside
//! the folder is ever opened, and no FIFO blocks the reader.
//!
//! Off unix (Windows, where isolation doesn't run) it falls back to checks by path.

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::path::{Component, Path};

/// What a folder entry is, seen without following it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    Symlink,
    Other,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: OsString,
    pub kind: Kind,
    /// Size in bytes from `lstat` at listing time. The file can grow after that: a reader caps
    /// what it reads from the open handle ([`read_capped`], [`copy_capped`]), never this.
    pub len: u64,
    /// Permission bits.
    pub mode: u32,
}

/// The plain parts of `rel`: refuses an absolute path, `..`, `.` and empty paths' oddities.
fn parts(rel: &Path) -> io::Result<Vec<&std::ffi::OsStr>> {
    rel.components()
        .map(|c| match c {
            Component::Normal(p) => Ok(p),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "not a plain relative path")),
        })
        .collect()
}

fn not_a_file() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "not a regular file")
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::ffi::{CStr, CString, OsStr};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    const DIR_FLAGS: libc::c_int = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    const FILE_FLAGS: libc::c_int = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;

    /// An open folder; everything under it is reached from this descriptor.
    #[derive(Debug)]
    pub struct Dir {
        fd: OwnedFd,
    }

    fn cstr(name: &OsStr) -> io::Result<CString> {
        CString::new(name.as_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in a name"))
    }

    /// A single plain name.
    fn one(name: &OsStr) -> io::Result<CString> {
        match parts(Path::new(name))?.as_slice() {
            [_] => cstr(name),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "not a single name")),
        }
    }

    fn check(fd: libc::c_int) -> io::Result<OwnedFd> {
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a descriptor the kernel just returned to us, owned by nothing else
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    // `mode_t` is u32 on Linux and u16 on macOS, so the casts are needed there
    #[allow(clippy::unnecessary_cast)]
    fn mode_kind(mode: u32) -> Kind {
        match mode & libc::S_IFMT as u32 {
            m if m == libc::S_IFREG as u32 => Kind::File,
            m if m == libc::S_IFDIR as u32 => Kind::Dir,
            m if m == libc::S_IFLNK as u32 => Kind::Symlink,
            _ => Kind::Other,
        }
    }

    fn fstat(fd: &OwnedFd) -> io::Result<libc::stat> {
        let mut st = std::mem::MaybeUninit::<libc::stat>::zeroed();
        // SAFETY: `st` is a valid out pointer for one `stat`
        if unsafe { libc::fstat(fd.as_raw_fd(), st.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fstat succeeded and filled it
        Ok(unsafe { st.assume_init() })
    }

    impl Dir {
        /// Opens `root` itself, resolved once: it's the person's own path (a symlink they made is
        /// fine) and, in vm mode, a mount point the bot can't swap. Everything below it is the
        /// bot's and is never followed.
        pub fn open(root: &Path) -> io::Result<Self> {
            let c = cstr(root.as_os_str())?;
            // SAFETY: a valid C string; the flags never create anything
            check(unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) }).map(|fd| Self { fd })
        }

        fn at(&self, name: &OsStr, flags: libc::c_int) -> io::Result<OwnedFd> {
            let c = cstr(name)?;
            // SAFETY: a valid descriptor and C string; the flags never create anything
            check(unsafe { libc::openat(self.fd.as_raw_fd(), c.as_ptr(), flags) })
        }

        /// The folder `rel` below this one (`""` is this folder again), every step a real folder.
        pub fn open_dir(&self, rel: &Path) -> io::Result<Dir> {
            let mut cur = Dir { fd: self.fd.try_clone()? };
            for p in parts(rel)? {
                cur = Dir { fd: cur.at(p, DIR_FLAGS)? };
            }
            Ok(cur)
        }

        /// The regular file `rel` below this folder, every folder on the way a real folder.
        pub fn open_file(&self, rel: &Path) -> io::Result<File> {
            let ps = parts(rel)?;
            let Some((last, dirs)) = ps.split_last() else { return Err(not_a_file()) };
            let mut cur = Dir { fd: self.fd.try_clone()? };
            for p in dirs {
                cur = Dir { fd: cur.at(p, DIR_FLAGS)? };
            }
            let fd = cur.at(last, FILE_FLAGS)?;
            if mode_kind(fstat(&fd)?.st_mode as u32) != Kind::File {
                return Err(not_a_file());
            }
            Ok(File::from(fd))
        }

        /// The folder `rel` below this one, made where missing (`mkdirat`), every step opened with
        /// `O_NOFOLLOW|O_DIRECTORY`: a symlink on the way is refused, never written through.
        pub fn make_dirs(&self, rel: &Path) -> io::Result<Dir> {
            let mut cur = Dir { fd: self.fd.try_clone()? };
            for p in parts(rel)? {
                let c = cstr(p)?;
                // SAFETY: a valid descriptor and C string
                if unsafe { libc::mkdirat(cur.fd.as_raw_fd(), c.as_ptr(), 0o777) } != 0 {
                    let e = io::Error::last_os_error();
                    if e.raw_os_error() != Some(libc::EEXIST) {
                        return Err(e);
                    }
                }
                cur = Dir { fd: cur.at(p, DIR_FLAGS)? };
            }
            Ok(cur)
        }

        /// Replaces the entry `name` in this folder with `src`'s bytes: written to a new `tmp`
        /// (`O_CREAT|O_EXCL|O_NOFOLLOW`), given `mode`, then `renameat` over `name` (a symlink there
        /// is replaced, not followed).
        pub fn replace_file(&self, name: &OsStr, tmp: &str, src: &mut impl io::Read, mode: u32) -> io::Result<()> {
            let (name, tmp) = (one(name)?, one(OsStr::new(tmp))?);
            let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
            // SAFETY: a valid descriptor and C string; O_EXCL never opens an existing entry
            let fd = check(unsafe { libc::openat(self.fd.as_raw_fd(), tmp.as_ptr(), flags, 0o600 as libc::c_uint) })?;
            let mut f = File::from(fd);
            let res = io::copy(src, &mut f).and_then(|_| {
                use std::os::unix::fs::PermissionsExt;
                f.set_permissions(std::fs::Permissions::from_mode(mode & 0o7777))
            });
            drop(f);
            // SAFETY: valid descriptors and C strings
            let res = res.and_then(|_| match unsafe { libc::renameat(self.fd.as_raw_fd(), tmp.as_ptr(), self.fd.as_raw_fd(), name.as_ptr()) } {
                0 => Ok(()),
                _ => Err(io::Error::last_os_error()),
            });
            if res.is_err() {
                // SAFETY: a valid descriptor and C string
                unsafe { libc::unlinkat(self.fd.as_raw_fd(), tmp.as_ptr(), 0) };
            }
            res
        }

        /// Removes the entry `name` (never a folder) from this folder.
        pub fn remove_file(&self, name: &OsStr) -> io::Result<()> {
            let c = one(name)?;
            // SAFETY: a valid descriptor and C string
            match unsafe { libc::unlinkat(self.fd.as_raw_fd(), c.as_ptr(), 0) } {
                0 => Ok(()),
                _ => Err(io::Error::last_os_error()),
            }
        }

        /// This folder's entries (without `.` and `..`), each seen with `fstatat(…,
        /// AT_SYMLINK_NOFOLLOW)` from this descriptor. An entry gone by then is left out.
        pub fn list(&self) -> io::Result<Vec<Entry>> {
            // fdopendir takes the descriptor over: give it a copy
            let dup = self.fd.try_clone()?;
            // SAFETY: a valid descriptor whose ownership passes to the DIR stream
            let stream = unsafe { libc::fdopendir(std::os::fd::IntoRawFd::into_raw_fd(dup)) };
            if stream.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut names = Vec::new();
            loop {
                // SAFETY: `stream` is a valid open DIR until closedir below
                let ent = unsafe { libc::readdir(stream) };
                if ent.is_null() {
                    break;
                }
                // SAFETY: readdir returned a valid entry whose name is NUL-terminated
                let name = unsafe { CStr::from_ptr((*ent).d_name.as_ptr()) }.to_bytes().to_vec();
                if name != b"." && name != b".." {
                    names.push(OsString::from_vec(name));
                }
            }
            // SAFETY: closes the stream (and the copied descriptor) once
            unsafe { libc::closedir(stream) };
            let mut out = Vec::with_capacity(names.len());
            for name in names {
                let Ok(c) = cstr(&name) else { continue };
                let mut st = std::mem::MaybeUninit::<libc::stat>::zeroed();
                // SAFETY: valid descriptor, C string and out pointer
                let rc = unsafe { libc::fstatat(self.fd.as_raw_fd(), c.as_ptr(), st.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) };
                if rc != 0 {
                    continue;
                }
                // SAFETY: fstatat succeeded and filled it
                let st = unsafe { st.assume_init() };
                #[allow(clippy::unnecessary_cast)] // u16 on macOS
                let mode = st.st_mode as u32;
                out.push(Entry { name, kind: mode_kind(mode), len: st.st_size.max(0) as u64, mode: mode & 0o7777 });
            }
            Ok(out)
        }
    }
}

#[cfg(not(unix))]
mod imp {
    use super::*;
    use std::path::PathBuf;

    /// Off unix: checks by path (isolation never runs here, so nobody races the reader).
    #[derive(Debug)]
    pub struct Dir {
        path: PathBuf,
    }

    fn real(p: &Path, dir: bool) -> io::Result<()> {
        // `open` resolves the root once; the parts below never follow
        let m = std::fs::symlink_metadata(p)?;
        let ok = if dir { m.is_dir() } else { m.is_file() };
        if ok {
            Ok(())
        } else {
            Err(not_a_file())
        }
    }

    impl Dir {
        pub fn open(root: &Path) -> io::Result<Self> {
            if !std::fs::metadata(root)?.is_dir() {
                return Err(not_a_file());
            }
            Ok(Self { path: root.to_path_buf() })
        }

        pub fn open_dir(&self, rel: &Path) -> io::Result<Dir> {
            let mut p = self.path.clone();
            for c in parts(rel)? {
                p.push(c);
                real(&p, true)?;
            }
            Ok(Dir { path: p })
        }

        pub fn open_file(&self, rel: &Path) -> io::Result<File> {
            let ps = parts(rel)?;
            let Some((last, dirs)) = ps.split_last() else { return Err(not_a_file()) };
            let mut p = self.path.clone();
            for c in dirs {
                p.push(c);
                real(&p, true)?;
            }
            p.push(last);
            real(&p, false)?;
            File::open(&p)
        }

        pub fn make_dirs(&self, rel: &Path) -> io::Result<Dir> {
            let mut p = self.path.clone();
            for c in parts(rel)? {
                p.push(c);
                if std::fs::symlink_metadata(&p).is_err() {
                    std::fs::create_dir(&p)?;
                }
                real(&p, true)?;
            }
            Ok(Dir { path: p })
        }

        pub fn replace_file(&self, name: &std::ffi::OsStr, tmp: &str, src: &mut impl io::Read, mode: u32) -> io::Result<()> {
            let _ = mode;
            let tmp = self.path.join(tmp);
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
            let res = io::copy(src, &mut f).map(|_| ());
            drop(f);
            let res = res.and_then(|_| std::fs::rename(&tmp, self.path.join(name)));
            if res.is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
            res
        }

        pub fn remove_file(&self, name: &std::ffi::OsStr) -> io::Result<()> {
            std::fs::remove_file(self.path.join(name))
        }

        pub fn list(&self) -> io::Result<Vec<Entry>> {
            let mut out = Vec::new();
            for e in std::fs::read_dir(&self.path)?.flatten() {
                let Ok(m) = std::fs::symlink_metadata(e.path()) else { continue };
                let ft = m.file_type();
                let kind = if ft.is_symlink() {
                    Kind::Symlink
                } else if ft.is_dir() {
                    Kind::Dir
                } else if ft.is_file() {
                    Kind::File
                } else {
                    Kind::Other
                };
                out.push(Entry { name: e.file_name(), kind, len: m.len(), mode: if m.permissions().readonly() { 0o444 } else { 0o644 } });
            }
            Ok(out)
        }
    }
}

pub use imp::Dir;

/// The error a capped reader gives for a file past its limit; see [`is_too_large`].
pub fn too_large() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "over the size limit")
}

/// Whether `e` is [`read_capped`]'s or [`copy_capped`]'s "over the size limit".
pub fn is_too_large(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::InvalidData && e.to_string() == "over the size limit"
}

/// Opens `real`, a path a caller already resolved and checked to lie under the resolved `root`,
/// by walking from `root`'s descriptor instead of opening it by path again (test plan S-8): if any
/// part became a symlink, a FIFO or anything but a regular file since the check, the open fails
/// instead of reading what it points at.
pub fn open_checked(root: &Path, real: &Path) -> io::Result<File> {
    let rel = real.strip_prefix(root).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "not under the folder"))?;
    Dir::open(root)?.open_file(rel)
}

/// All of `r`, refused once it passes `cap` bytes: a size seen before the open is never trusted,
/// since the file can grow after it.
pub fn read_capped(r: impl io::Read, cap: u64) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    io::Read::read_to_end(&mut r.take(cap.saturating_add(1)), &mut out)?;
    if out.len() as u64 > cap {
        return Err(too_large());
    }
    Ok(out)
}

/// Copies `r` into `w` (a hasher, a file), refused once it passes `cap` bytes; the bytes copied.
pub fn copy_capped(r: impl io::Read, w: &mut impl io::Write, cap: u64) -> io::Result<u64> {
    let n = io::copy(&mut r.take(cap.saturating_add(1)), w)?;
    if n > cap {
        return Err(too_large());
    }
    Ok(n)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::fs::symlink;

    fn read(f: io::Result<File>) -> io::Result<String> {
        let mut s = String::new();
        f?.read_to_string(&mut s)?;
        Ok(s)
    }

    #[test]
    fn reads_real_files_and_refuses_every_symlink_on_the_way() {
        let t = tempfile::tempdir().unwrap();
        let (root, outside) = (t.path().join("root"), t.path().join("outside"));
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("a/b/f.txt"), "inside").unwrap();
        std::fs::write(outside.join("secret"), "SECRET").unwrap();
        symlink(&outside, root.join("a/link_dir")).unwrap();
        symlink(outside.join("secret"), root.join("a/link_file")).unwrap();
        let d = Dir::open(&root).unwrap();
        assert_eq!(read(d.open_file(Path::new("a/b/f.txt"))).unwrap(), "inside");
        assert!(read(d.open_file(Path::new("a/link_file"))).is_err());
        assert!(read(d.open_file(Path::new("a/link_dir/secret"))).is_err());
        assert!(d.open_dir(Path::new("a/link_dir")).is_err());
        for bad in ["../outside/secret", "/etc/passwd", "a/../a/b/f.txt", ""] {
            assert!(d.open_file(Path::new(bad)).is_err(), "{bad}");
        }
        // The root is the person's path and is resolved once; only what's below it is never followed
        symlink(&root, t.path().join("root_link")).unwrap();
        assert_eq!(read(Dir::open(&t.path().join("root_link")).unwrap().open_file(Path::new("a/b/f.txt"))).unwrap(), "inside");
        let mut names: Vec<(String, Kind)> =
            d.open_dir(Path::new("a")).unwrap().list().unwrap().into_iter().map(|e| (e.name.to_string_lossy().into_owned(), e.kind)).collect();
        names.sort_by(|x, y| x.0.cmp(&y.0));
        assert_eq!(names, vec![("b".into(), Kind::Dir), ("link_dir".into(), Kind::Symlink), ("link_file".into(), Kind::Symlink)]);
    }

    #[test]
    fn writes_never_go_through_a_symlinked_folder() {
        let t = tempfile::tempdir().unwrap();
        let (root, outside) = (t.path().join("root"), t.path().join("outside"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let d = Dir::open(&root).unwrap();
        let sub = d.make_dirs(Path::new("a/b")).unwrap();
        sub.replace_file("f".as_ref(), ".tmp-1", &mut "new".as_bytes(), 0o640).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("a/b/f")).unwrap(), "new");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(root.join("a/b/f")).unwrap().permissions().mode() & 0o777, 0o640);
        // A symlinked folder on the way is refused for writing too
        symlink(&outside, root.join("a/evil")).unwrap();
        assert!(d.make_dirs(Path::new("a/evil/x")).is_err());
        // A symlink in the file's place is replaced, its target untouched
        std::fs::write(outside.join("target"), "keep").unwrap();
        symlink(outside.join("target"), root.join("a/b/g")).unwrap();
        sub.replace_file("g".as_ref(), ".tmp-2", &mut "mine".as_bytes(), 0o644).unwrap();
        assert_eq!(std::fs::read_to_string(outside.join("target")).unwrap(), "keep");
        assert!(std::fs::symlink_metadata(root.join("a/b/g")).unwrap().is_file());
        // Swapped after the folder was opened: the write lands in the real (moved) folder
        std::fs::rename(root.join("a/b"), root.join("a/b.moved")).unwrap();
        symlink(&outside, root.join("a/b")).unwrap();
        sub.replace_file("h".as_ref(), ".tmp-3", &mut "x".as_bytes(), 0o644).unwrap();
        assert!(!outside.join("h").exists() && root.join("a/b.moved/h").exists());
        assert!(sub.replace_file("../h".as_ref(), ".tmp-4", &mut "x".as_bytes(), 0o644).is_err());
        sub.remove_file("h".as_ref()).unwrap();
        assert!(d.open_dir(Path::new("a/b")).is_err());
    }

    #[test]
    fn a_fifo_is_refused_without_blocking() {
        let t = tempfile::tempdir().unwrap();
        let c = std::ffi::CString::new(t.path().join("pipe").to_string_lossy().as_bytes()).unwrap();
        // SAFETY: a valid C string
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let d = Dir::open(t.path()).unwrap();
        assert!(d.open_file(Path::new("pipe")).is_err());
    }

    /// S-8: the folder is swapped for a symlink between calls. A held folder keeps reading the
    /// folder it opened (now moved aside), never the symlink's target; a fresh walk refuses it.
    #[test]
    fn a_folder_swapped_for_a_symlink_between_calls_never_reaches_outside() {
        let t = tempfile::tempdir().unwrap();
        let (root, outside) = (t.path().join("root"), t.path().join("outside"));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("sub/notes.txt"), "mine").unwrap();
        std::fs::write(outside.join("notes.txt"), "SECRET").unwrap();
        let d = Dir::open(&root).unwrap();
        let sub = d.open_dir(Path::new("sub")).unwrap();
        // The bot swaps `sub` for a symlink to a folder outside
        std::fs::rename(root.join("sub"), root.join("sub.moved")).unwrap();
        symlink(&outside, root.join("sub")).unwrap();
        assert!(read(d.open_file(Path::new("sub/notes.txt"))).is_err(), "a fresh walk refuses the symlink");
        assert_eq!(read(sub.open_file(Path::new("notes.txt"))).unwrap(), "mine", "the held folder is still the real one");
        assert!(sub.list().unwrap().iter().all(|e| e.name != "SECRET"));
        // And the same for the file itself
        std::fs::remove_file(root.join("sub")).unwrap();
        std::fs::create_dir(root.join("sub")).unwrap();
        symlink(outside.join("notes.txt"), root.join("sub/notes.txt")).unwrap();
        assert!(read(d.open_file(Path::new("sub/notes.txt"))).is_err());
    }

    /// The race itself, many times: another thread keeps swapping a folder and a symlink while
    /// this one reads through it. Every read is the real file or an error, never the target.
    #[test]
    fn a_reader_racing_a_swapper_never_reads_the_target() {
        let t = tempfile::tempdir().unwrap();
        let (root, outside) = (t.path().join("root"), t.path().join("outside"));
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("real/f"), "mine").unwrap();
        std::fs::write(outside.join("f"), "SECRET").unwrap();
        symlink(&outside, root.join("evil")).unwrap();
        symlink(root.join("real"), root.join("evil_inside")).unwrap();
        std::fs::rename(root.join("real"), root.join("x")).unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let swapper = {
            let (root, stop) = (root.clone(), stop.clone());
            std::thread::spawn(move || {
                // `x` is the real folder or the symlink to outside, swapped with rename(2)
                let mut real = true;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let (from, to) = if real { ("x", "real_parked") } else { ("real_parked", "x") };
                    let _ = std::fs::rename(root.join(from), root.join(to));
                    if real {
                        let _ = std::fs::rename(root.join("evil"), root.join("x"));
                    } else {
                        let _ = std::fs::rename(root.join("x"), root.join("evil"));
                        let _ = std::fs::rename(root.join("real_parked"), root.join("x"));
                    }
                    real = !real;
                }
            })
        };
        let d = Dir::open(&root).unwrap();
        let mut ok = 0;
        for _ in 0..5000 {
            if let Ok(s) = read(d.open_file(Path::new("x/f"))) {
                assert_eq!(s, "mine");
                ok += 1;
            }
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        swapper.join().unwrap();
        // Not a requirement, just that the race really ran both ways
        eprintln!("{ok} of 5000 reads saw the real folder");
    }
}
