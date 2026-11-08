//! Windows only: one Job Object per bot turn, so Stop ends the whole process tree (plan decision 3).
//!
//! Windows has no process groups. Every process a job member starts joins the same job, so
//! terminating the job ends the CLI or bot command, its children and their children. The job is
//! created with `KILL_ON_JOB_CLOSE`, so even if kwnt itself dies, closing its handle ends them.

use std::ffi::c_void;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

pub(crate) struct Job(HANDLE);

// SAFETY: a job handle is a kernel handle; any thread may use or close it.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Job {
    /// A new kill-on-close job holding `process` (a raw process handle), or None if Windows refused.
    pub(crate) fn holding(process: *mut c_void) -> Option<Self> {
        // SAFETY: plain Win32 calls on handles this function owns or was given; the info struct
        // lives on the stack for the duration of the call and its size is passed with it.
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return None;
            }
            let job = Job(handle);
            if !job.set_kill_on_close(true) {
                return None;
            }
            if AssignProcessToJobObject(job.0, process as HANDLE) == 0 {
                return None;
            }
            Some(job)
        }
    }

    fn set_kill_on_close(&self, on: bool) -> bool {
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        if on {
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        }
        // SAFETY: see `holding`
        unsafe {
            SetInformationJobObject(
                self.0,
                JobObjectExtendedLimitInformation,
                &info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION as *const c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) != 0
        }
    }

    /// Ends every process in the job now.
    pub(crate) fn kill(&self) {
        // SAFETY: a valid job handle owned by self
        unsafe {
            TerminateJobObject(self.0, 1);
        }
    }

    /// Lets the job's processes outlive it: clears kill-on-close, then closes the handle.
    pub(crate) fn release(self) {
        self.set_kill_on_close(false);
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by self and closed exactly once
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// Fallback when there is no job: `taskkill /T /F` ends `pid` and every process it started.
pub(crate) fn taskkill_tree(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}
