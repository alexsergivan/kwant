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
