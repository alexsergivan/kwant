/// The app's own commands (connect.rs). Listing them turns on Tauri's access control for them:
/// only capabilities/connect.json grants them, and it is local-only.
const COMMANDS: &[&str] = &[
    "places_state",
    "check_server",
    "add_server",
    "remove_server",
    "set_this_mac",
    "close_connect",
    "retry_start",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("tauri build");
}
