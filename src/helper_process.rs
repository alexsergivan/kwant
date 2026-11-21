//! kwnt's own short helper commands (`docker`, `where`), run to the end like `Command::output`.
//!
//! On Windows each one is put in a kill-on-close Job Object that kwnt holds for its whole life
//! ([`crate::drivers::win_job::hold_helper`]), so a helper still running when kwnt.exe is killed
//! outright ends with it. Windows never ends or re-parents orphans, and tokio's `kill_on_drop`
//! doesn't run on a forced kill, so a `docker` stuck on a slow daemon would otherwise be left
//! behind with its console. On Mac and Linux these are plain `output()` and `status()`.

use std::io;
use std::process::{ExitStatus, Output};

/// `cmd.output().await`, with the helper held by kwnt's helper job on Windows. As with `output()`,
/// stdin is null and stdout and stderr are captured; `kill_on_drop` and the rest of `cmd` apply.
pub(crate) async fn output(cmd: &mut tokio::process::Command) -> io::Result<Output> {
    #[cfg(windows)]
    {
        use std::process::Stdio;
        let child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        if let Some(handle) = child.raw_handle() {
            crate::drivers::win_job::hold_helper(handle);
        }
        child.wait_with_output().await
    }
    #[cfg(not(windows))]
    {
        cmd.output().await
    }
}

/// `cmd.output()` (blocking), with the helper held by kwnt's helper job. As with `output()`, stdin
/// is null and stdout and stderr are captured. Windows only: its one caller is the `where` lookup.
#[cfg(windows)]
pub(crate) fn output_blocking(cmd: &mut std::process::Command) -> io::Result<Output> {
    use std::os::windows::io::AsRawHandle;
    use std::process::Stdio;
    let child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    crate::drivers::win_job::hold_helper(child.as_raw_handle());
    child.wait_with_output()
}

/// `cmd.status()` (blocking), with the helper held by kwnt's helper job on Windows. The stdio
/// `cmd` sets is kept; unset, it is inherited, as with `status()`.
pub(crate) fn status_blocking(cmd: &mut std::process::Command) -> io::Result<ExitStatus> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        let mut child = cmd.spawn()?;
        crate::drivers::win_job::hold_helper(child.as_raw_handle());
        child.wait()
    }
    #[cfg(not(windows))]
    {
        cmd.status()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo_hi() -> (&'static str, Vec<&'static str>) {
        if cfg!(windows) {
            ("cmd", vec!["/c", "echo hi& echo err 1>&2& exit /b 3"])
        } else {
            ("sh", vec!["-c", "echo hi; echo err >&2; exit 3"])
        }
    }

    #[tokio::test]
    async fn output_captures_stdout_stderr_and_the_exit_code() {
        let (program, args) = echo_hi();
        let out = output(tokio::process::Command::new(program).args(&args).kill_on_drop(true)).await.unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
        assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "err");
        assert_eq!(out.status.code(), Some(3));
    }

    #[test]
    fn the_blocking_ones_match_output_and_status() {
        let (program, args) = echo_hi();
        #[cfg(windows)]
        {
            let out = output_blocking(std::process::Command::new(program).args(&args)).unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
            assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "err");
            assert_eq!(out.status.code(), Some(3));
        }
        let status = status_blocking(
            std::process::Command::new(program).args(&args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()),
        )
        .unwrap();
        assert_eq!(status.code(), Some(3));
    }
}
