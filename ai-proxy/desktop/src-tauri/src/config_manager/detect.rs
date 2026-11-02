use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
pub struct DetectedTool {
    pub tool: String,
    pub path: PathBuf,
    pub config_path: PathBuf,
}

pub fn detect_all() -> Vec<DetectedTool> {
    let home = home_dir();
    let dirs = search_dirs();
    let mut out = Vec::new();
    if let Some(p) = find("claude", &dirs) {
        out.push(DetectedTool {
            tool: "claude".into(),
            path: p,
            config_path: home.join(".claude/settings.json"),
        });
    }
    if let Some(p) = find("codex", &dirs) {
        out.push(DetectedTool {
            tool: "codex".into(),
            path: p,
            config_path: home.join(".codex/config.toml"),
        });
    }
    out
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Ok(out) = Command::new("zsh").args(["-lic", "printf %s \"$PATH\""]).output() {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            dirs.extend(std::env::split_paths(text.as_ref()));
        }
    }
    let home = home_dir();
    dirs.extend([
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        home.join(".local/bin"),
        home.join(".npm-global/bin"),
    ]);
    dirs
}

pub fn find(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter().map(|d| d.join(name)).find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata()
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_executable_in_dir() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("claude");
        std::fs::write(&f, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&f).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&f, perms).unwrap();
        }
        let found = find("claude", &[dir.path().to_path_buf()]);
        assert_eq!(found, Some(f));
    }
}
