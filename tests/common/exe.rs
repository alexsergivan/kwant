//! Fake CLIs that a test writes and then runs.
//!
//! `std::fs::write` followed by running the file is racy in a test binary: while this process
//! has the file open for writing, any other test thread that spawns a process forks a child
//! holding a copy of that write descriptor until the child execs, and running a file that some
//! process still has open for writing fails with ETXTBSY ("Text file busy"). So the bytes go to a
//! sibling file and `install`, its own process whose descriptors no fork of ours can inherit,
//! makes the executable, which is then renamed into place (a new inode, even when replacing).

use std::path::{Path, PathBuf};

pub fn write_executable(path: impl AsRef<Path>, body: impl AsRef<[u8]>) {
    let path = path.as_ref();
    let with_suffix = |s: &str| -> PathBuf {
        let mut p = path.as_os_str().to_owned();
        p.push(s);
        p.into()
    };
    let (src, staged) = (with_suffix(".body"), with_suffix(".staged"));
    std::fs::write(&src, body).unwrap();
    let status = std::process::Command::new("install").arg("-m").arg("755").arg(&src).arg(&staged).status().unwrap();
    assert!(status.success(), "install {} failed: {status}", staged.display());
    std::fs::rename(&staged, path).unwrap();
    std::fs::remove_file(&src).unwrap();
}
