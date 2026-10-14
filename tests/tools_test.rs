use kwnt::tools::ToolRunner;
use serde_json::json;
use tempfile::tempdir;

#[tokio::test]
async fn test_path_traversal_is_blocked() {
    let tmp = tempdir().unwrap();
    let runner = ToolRunner::new(tmp.path().to_path_buf());

    let bad_args = json!({
        "path": "../../etc/shadow",
        "content": "hacked"
    });

    let res = runner.execute("write_file", &bad_args).await;
    assert!(res.is_err());
    let err_str = res.unwrap_err().to_string();
    assert!(err_str.contains("Path traversal") || err_str.contains("outside"));
}

#[tokio::test]
async fn test_safe_file_lifecycle() {
    let tmp = tempdir().unwrap();
    let runner = ToolRunner::new(tmp.path().to_path_buf());

    let write_args = json!({
        "path": "hero_stats.txt",
        "content": "HP: 100\nMP: 50\nLVL: 5"
    });
    let write_res = runner.execute("write_file", &write_args).await.unwrap();
    assert_eq!(write_res["status"], "success");

    let read_args = json!({
        "path": "hero_stats.txt"
    });
    let read_res = runner.execute("read_file", &read_args).await.unwrap();
    assert_eq!(read_res["content"], "HP: 100\nMP: 50\nLVL: 5");

    let list_args = json!({ "path": "." });
    let list_res = runner.execute("list_dir", &list_args).await.unwrap();
    let entries = list_res["entries"].as_array().unwrap();
    assert!(entries.iter().any(|e| e["name"] == "hero_stats.txt"));
}

#[tokio::test]
async fn test_shell_execution() {
    let tmp = tempdir().unwrap();
    let runner = ToolRunner::new(tmp.path().to_path_buf());

    let shell_args = json!({
        "command": "echo 'LEVEL UP'"
    });
    let res = runner.execute("execute_shell", &shell_args).await.unwrap();
    assert!(res["stdout"].as_str().unwrap().contains("LEVEL UP"));
    assert_eq!(res["exit_code"], 0);
}
