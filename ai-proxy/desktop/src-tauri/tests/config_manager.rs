use ai_proxy_desktop_lib::config_manager::{claude, codex};
use std::path::Path;

fn bak_files(home: &Path, name: &str) -> usize {
    std::fs::read_dir(home)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(name))
        .count()
}

#[test]
fn claude_takeover_backs_up_and_preserves_other_keys() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::write(
        home.join(".claude/settings.json"),
        r#"{"theme":"dark","env":{"FOO":"bar"}}"#,
    )
    .unwrap();
    let backup = claude::takeover(home, "http://127.0.0.1:8787", "tok").unwrap().unwrap();
    assert!(backup.exists());
    let v: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.join(".claude/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(v["theme"], "dark");
    assert_eq!(v["env"]["FOO"], "bar");
    assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:8787");
    assert_eq!(v["env"]["ANTHROPIC_AUTH_TOKEN"], "tok");
}

#[test]
fn claude_takeover_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::write(home.join(".claude/settings.json"), "{}").unwrap();
    claude::takeover(home, "http://127.0.0.1:8787", "tok1").unwrap().unwrap();
    assert!(claude::takeover(home, "http://127.0.0.1:8787", "tok2").unwrap().is_none());
    assert_eq!(bak_files(&home.join(".claude"), "settings.json.bak-"), 1);
    let v: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.join(".claude/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(v["env"]["ANTHROPIC_AUTH_TOKEN"], "tok2");
}

#[test]
fn claude_restore_returns_original() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    let original = r#"{"theme":"dark"}"#;
    std::fs::write(home.join(".claude/settings.json"), original).unwrap();
    let backup = claude::takeover(home, "http://127.0.0.1:8787", "tok").unwrap().unwrap();
    claude::restore(home, &backup).unwrap();
    assert_eq!(
        std::fs::read_to_string(home.join(".claude/settings.json")).unwrap(),
        original
    );
}

#[test]
fn claude_bad_file_aborts_without_change() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::write(home.join(".claude/settings.json"), "not-json").unwrap();
    let err = claude::takeover(home, "http://127.0.0.1:8787", "tok").unwrap_err();
    assert!(err.contains("解析失败"));
    assert_eq!(
        std::fs::read_to_string(home.join(".claude/settings.json")).unwrap(),
        "not-json"
    );
}

#[test]
fn codex_takeover_sets_provider_and_backs_up() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::write(
        home.join(".codex/config.toml"),
        "model = \"o3\"\napproval_policy = \"on-request\"\n",
    )
    .unwrap();
    let backup = codex::takeover(home, "http://127.0.0.1:8787/v1", "tok").unwrap().unwrap();
    assert!(backup.exists());
    let text = std::fs::read_to_string(home.join(".codex/config.toml")).unwrap();
    assert!(text.contains("[model_providers.bridge]"));
    assert!(text.contains("base_url = \"http://127.0.0.1:8787/v1\""));
    assert!(text.contains("env_key = \"DEEPSEEK_BRIDGE_TOKEN\""));
    assert!(text.contains("model = \"bridge\""));
    assert!(text.contains("approval_policy = \"on-request\""));
}

#[test]
fn codex_restore_returns_original() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    let original = "model = \"o3\"\n";
    std::fs::write(home.join(".codex/config.toml"), original).unwrap();
    let backup = codex::takeover(home, "http://127.0.0.1:8787/v1", "tok").unwrap().unwrap();
    codex::restore(home, &backup).unwrap();
    assert_eq!(
        std::fs::read_to_string(home.join(".codex/config.toml")).unwrap(),
        original
    );
}
