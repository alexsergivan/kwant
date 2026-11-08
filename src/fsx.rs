//! File modes that work on every platform kwant runs on.
//!
//! On Mac and Linux these are the usual Unix modes (0600 files, 0700 folders), set exactly as
//! before. Windows has no mode bits: everything private lives under the data dir
//! (`%USERPROFILE%\.kwnt`), whose access list [`restrict_to_user`] limits to the current user and
//! SYSTEM, and every file and folder under it inherits that list. So the mode calls are no-ops
//! there.

use std::path::Path;

/// `mode` on Unix (`OpenOptionsExt::mode`, `DirBuilderExt::mode`); nothing on Windows, where the
/// data dir's inherited access list does the job.
pub trait UnixMode {
    fn unix_mode(&mut self, mode: u32) -> &mut Self;
}

impl UnixMode for std::fs::OpenOptions {
    fn unix_mode(&mut self, mode: u32) -> &mut Self {
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(self, mode);
        #[cfg(not(unix))]
        let _ = mode;
        self
    }
}

impl UnixMode for std::fs::DirBuilder {
    fn unix_mode(&mut self, mode: u32) -> &mut Self {
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(self, mode);
        #[cfg(not(unix))]
        let _ = mode;
        self
    }
}

/// `chmod` on Unix; nothing on Windows (see the module note).
pub fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(())
    }
}

/// The permission bits of a file (`mode & 0o777`). Windows has only read-only or not: 0444 or
/// 0644.
pub fn mode_of(meta: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        if meta.permissions().readonly() {
            0o444
        } else {
            0o644
        }
    }
}

/// SYSTEM's well-known SID, the same in every Windows language.
#[cfg(windows)]
pub const SYSTEM_SID: &str = "S-1-5-18";

/// The current user's SID (`S-1-5-21-…`), from `whoami /user`.
#[cfg(windows)]
pub fn current_user_sid() -> Option<String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("whoami")
        .args(["/user", "/fo", "csv", "/nh"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    sid_from_whoami(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The SID in `whoami /user /fo csv /nh` output: `"pc\name","S-1-5-21-…"`.
#[cfg_attr(not(windows), allow(dead_code))]
fn sid_from_whoami(text: &str) -> Option<String> {
    let sid = text.trim().rsplit(',').next()?.trim().trim_matches('"');
    let ok = sid.starts_with("S-1-") && sid.len() > 4 && sid[4..].chars().all(|c| c.is_ascii_digit() || c == '-');
    ok.then(|| sid.to_string())
}

/// Limits `dir` to the current user and SYSTEM, for the folders and everything below them
/// (Windows: no inherited entries, `(OI)(CI)F` for the two; plan decision 4). Mac and Linux
/// set 0700 on the data dir elsewhere, so this does nothing there. A failure is logged as a
/// warning and never stops kwant.
pub fn restrict_to_user(dir: &Path) {
    #[cfg(windows)]
    if let Err(e) = restrict_windows(dir) {
        tracing::warn!("could not limit {} to this user: {e}", dir.display());
    }
    #[cfg(not(windows))]
    let _ = dir;
}

#[cfg(windows)]
fn restrict_windows(dir: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let sid = current_user_sid().ok_or("no user SID from whoami")?;
    // `*SID` names an account by SID, so a localized "SYSTEM" or a domain user never matters
    let out = std::process::Command::new("icacls")
        .arg(dir)
        .args(["/inheritance:r", "/grant:r"])
        .arg(format!("*{sid}:(OI)(CI)F"))
        .arg(format!("*{SYSTEM_SID}:(OI)(CI)F"))
        .arg("/q")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("icacls: {}", e.kind()))?;
    icacls_answer(out)?;
    // `/grant:r` leaves other explicit entries; a folder made by an elevated process carries one for
    // BUILTIN\Administrators from the token's default access list, so drop it by its SID
    let out = std::process::Command::new("icacls")
        .arg(dir)
        .args(["/remove:g", &format!("*{ADMINISTRATORS_SID}"), "/q"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("icacls: {}", e.kind()))?;
    icacls_answer(out)
}

#[cfg(windows)]
const ADMINISTRATORS_SID: &str = "S-1-5-32-544";

#[cfg(windows)]
fn icacls_answer(out: std::process::Output) -> Result<(), String> {
    if out.status.success() {
        Ok(())
    } else {
        let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        Err(format!("icacls exited with {}: {}", out.status, said.trim().chars().take(300).collect::<String>()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whoami_csv_gives_the_sid() {
        assert_eq!(
            sid_from_whoami("\"desktop-1\\alex\",\"S-1-5-21-1004336348-1177238915-682003330-1001\"\r\n").as_deref(),
            Some("S-1-5-21-1004336348-1177238915-682003330-1001")
        );
        assert_eq!(sid_from_whoami("ERROR: no"), None);
        assert_eq!(sid_from_whoami("\"x\",\"S-1-\""), None);
        assert_eq!(sid_from_whoami("\"x\",\"S-1-5-21 & calc\""), None);
    }

    #[test]
    fn modes_round_trip_where_there_are_modes() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("f");
        let mut o = std::fs::OpenOptions::new();
        let mut file = o.write(true).create_new(true).unix_mode(0o600).open(&f).unwrap();
        std::io::Write::write_all(&mut file, b"x").unwrap();
        drop(file);
        let m = mode_of(&std::fs::metadata(&f).unwrap());
        if cfg!(unix) {
            assert_eq!(m, 0o600);
            set_mode(&f, 0o640).unwrap();
            assert_eq!(mode_of(&std::fs::metadata(&f).unwrap()), 0o640);
        } else {
            assert_eq!(m, 0o644);
            set_mode(&f, 0o640).unwrap();
        }
        std::fs::DirBuilder::new().unix_mode(0o700).create(tmp.path().join("d")).unwrap();
        assert!(tmp.path().join("d").is_dir());
    }

    /// The SIDs on `path`'s access list, and whether any entry is inherited, read back with
    /// PowerShell (`Get-Acl`), independent of how `icacls` wrote them.
    #[cfg(windows)]
    fn acl_sids(path: &Path) -> (Vec<String>, bool) {
        let script = format!(
            "(Get-Acl -LiteralPath '{}').Access | ForEach-Object {{ $_.IdentityReference.Translate([System.Security.Principal.SecurityIdentifier]).Value + ' ' + $_.IsInherited }}",
            path.display().to_string().replace('\'', "''")
        );
        // PowerShell 7 (the CI's shell) leaves its own module path behind, which Windows
        // PowerShell can't load Get-Acl from; without it Windows PowerShell uses its default
        let out = std::process::Command::new("powershell")
            .env_remove("PSModulePath")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .unwrap();
        assert!(out.status.success(), "Get-Acl: {}", String::from_utf8_lossy(&out.stderr));
        let text = String::from_utf8_lossy(&out.stdout);
        let mut sids = Vec::new();
        let mut inherited = false;
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let (sid, inh) = line.split_once(' ').unwrap();
            sids.push(sid.to_string());
            inherited |= inh.eq_ignore_ascii_case("true");
        }
        sids.sort();
        sids.dedup();
        (sids, inherited)
    }

    /// Plan decision 4: `.kwnt` is the current user's and SYSTEM's only, and so is a key file
    /// that was already in it.
    #[cfg(windows)]
    #[test]
    fn the_data_dir_is_limited_to_this_user_and_system() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".kwnt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("api-token"), "t").unwrap();
        let user = current_user_sid().expect("whoami gives a SID");
        let before = acl_sids(&dir).0;
        assert!(before.len() > 2, "a temp folder starts with more than two entries: {before:?}");
        restrict_windows(&dir).expect("icacls limits the folder");
        let mut want = vec![user, SYSTEM_SID.to_string()];
        want.sort();
        let (sids, inherited) = acl_sids(&dir);
        assert!(!inherited, "no entry inherited from the profile: {sids:?}");
        assert_eq!(sids, want, "the folder");
        // The file inherits the folder's two entries, nothing else
        let (file_sids, _) = acl_sids(&dir.join("api-token"));
        assert_eq!(file_sids, want, "a file already in it");
        // ...and so does one made afterwards
        std::fs::write(dir.join("later"), "t").unwrap();
        assert_eq!(acl_sids(&dir.join("later")).0, want, "a file made afterwards");
    }
}
