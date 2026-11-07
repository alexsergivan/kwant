//! kwnt reports the release's version (0.7.6, O8): `kwnt --version` and `/api/health` read the root
//! crate's version, which a release bumps together with the Mac app's (the tag check in mac.yml
//! fails otherwise). Before 0.7.6 the root crate stayed at 0.1.0 and both said "0.1.0".

use std::path::Path;

fn manifest_version(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap();
    let doc: toml::Value = toml::from_str(&text).unwrap();
    doc["package"]["version"].as_str().unwrap().to_string()
}

#[test]
fn kwnt_has_the_mac_apps_release_version() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mac = manifest_version(&root.join("apps/mac/Cargo.toml"));
    let tauri: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("apps/mac/tauri.conf.json")).unwrap()).unwrap();
    assert_eq!(env!("CARGO_PKG_VERSION"), mac, "bump the root Cargo.toml with apps/mac/Cargo.toml");
    assert_eq!(tauri["version"], mac);
    assert_ne!(env!("CARGO_PKG_VERSION"), "0.1.0");
}

#[test]
fn both_lockfiles_carry_the_same_kwnt_version() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for lock in ["Cargo.lock", "apps/mac/Cargo.lock"] {
        let text = std::fs::read_to_string(root.join(lock)).unwrap();
        let doc: toml::Value = toml::from_str(&text).unwrap();
        let kwnt = doc["package"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"].as_str() == Some("kwnt"))
            .unwrap_or_else(|| panic!("no kwnt in {lock}"));
        assert_eq!(kwnt["version"].as_str(), Some(env!("CARGO_PKG_VERSION")), "{lock}");
    }
}
