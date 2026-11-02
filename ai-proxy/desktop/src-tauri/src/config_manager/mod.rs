pub mod claude;
pub mod codex;
pub mod detect;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn backup(path: &Path) -> io::Result<PathBuf> {
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let dest = PathBuf::from(format!("{}.bak-{}", path.display(), ts));
    fs::copy(path, &dest)?;
    Ok(dest)
}
