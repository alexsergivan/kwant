//! D4c / S-13 (agents in the VM): kwant starts git on this computer only through
//! `server::dev::git::host_git_command`, which forces `GIT_CONFIG_NOSYSTEM=1` and turns off
//! hooks, fsmonitor and the pager. This fails when anything else in `src/` (outside test code)
//! starts git itself.

use std::path::Path;

fn rs_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The file without its `#[cfg(test)]` modules (everything from the first one on; kwant keeps
/// its unit tests at the end of each file).
fn non_test(src: &str) -> &str {
    src.find("#[cfg(test)]").map_or(src, |i| &src[..i])
}

#[test]
fn only_the_helper_starts_git() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rs_files(&root, &mut files);
    let mut found = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        let code = non_test(&text);
        for (n, line) in code.lines().enumerate() {
            let l = line.split("//").next().unwrap_or("");
            let starts_git = l.contains("Command::new(\"git\")") || l.contains("Command::new(\"git.exe\")");
            let helper = f.ends_with("server/dev/git.rs") && l.contains("Command::new(bin)");
            if starts_git || helper {
                found.push(format!("{}:{}", f.strip_prefix(&root).unwrap().display(), n + 1));
            }
        }
    }
    // Exactly one: the line inside `host_git_command`
    assert_eq!(found.len(), 1, "git started outside host_git_command: {found:?}");
    assert!(found[0].starts_with("server/dev/git.rs:"), "{found:?}");
}
