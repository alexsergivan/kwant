//! Stops the Chrome a test started on its profile when the test ends, also when an assert fails
//! first: a test's own cleanup at its end never runs then, and the Chrome, detached from the test,
//! would keep running with nothing to close it.

use std::path::PathBuf;

pub struct ChromeGuard(pub PathBuf);

impl Drop for ChromeGuard {
    fn drop(&mut self) {
        // Nothing when the test closed its Chrome itself
        if let Some(pid) = kwnt::sandbox::computer::profile_owner(&self.0) {
            // SAFETY: kill(2) with a pid that profile_owner just saw as this profile's Chrome
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
        }
    }
}
