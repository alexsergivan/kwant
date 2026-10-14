#[test]
fn test_default_config() {
    let cfg = kwnt::config::Config::default();
    assert_eq!(cfg.port, 8799);
    assert_eq!(cfg.host, "127.0.0.1");
    assert_eq!(cfg.laya_url, "http://127.0.0.1:8000");
    assert!(cfg.data_dir.ends_with(".kwnt"));
}

#[test]
fn test_env_override() {
    std::env::set_var("KWNT_PORT", "9999");
    std::env::set_var("LAYA_URL", "http://router.local:8000");
    let cfg = kwnt::config::Config::from_env_or_default();
    assert_eq!(cfg.port, 9999);
    assert_eq!(cfg.laya_url, "http://router.local:8000");
}
