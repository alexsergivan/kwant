use std::path::Path;

#[test]
fn test_docker_packaging_files_exist_and_valid() {
    let dockerfile = Path::new("docker/sandbox.Dockerfile");
    let entrypoint = Path::new("docker/entrypoint.sh");

    assert!(dockerfile.exists(), "docker/sandbox.Dockerfile must exist");
    assert!(entrypoint.exists(), "docker/entrypoint.sh must exist");

    let df_content = std::fs::read_to_string(dockerfile).unwrap();
    assert!(df_content.contains("FROM debian:bookworm-slim"));
    assert!(df_content.contains("chromium"));
    assert!(df_content.contains("EXPOSE 9222"));

    let ep_content = std::fs::read_to_string(entrypoint).unwrap();
    assert!(ep_content.contains("--remote-debugging-port=9222"));
    assert!(ep_content.contains("--headless"));
}

/// Scripts copied into images keep the host file mode, which a strict umask can make 0600.
/// `chmod +x` alone then leaves them unreadable to a non-root USER, so every image sets 755.
#[test]
fn test_copied_entrypoints_are_world_readable() {
    for (dockerfile, script) in [
        ("docker/browser.Dockerfile", "/usr/local/bin/browser-entrypoint"),
        ("docker/sandbox.Dockerfile", "/entrypoint.sh"),
    ] {
        let content = std::fs::read_to_string(dockerfile).unwrap();
        assert!(content.contains(&format!("chmod 755 {script}")), "{dockerfile} must chmod 755 {script}");
        assert!(!content.contains("chmod +x"), "{dockerfile} must not rely on chmod +x alone");
    }
}
