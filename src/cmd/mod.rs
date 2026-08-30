pub mod image;
pub mod podcast;
pub mod rewrite;
pub mod run;
pub mod video;

use std::path::{Path, PathBuf};

/// 把 --id 解析为任务目录：
/// 1) id 是已存在的目录（相对/绝对路径）→ 直接用
/// 2) 标准情况 output/<id>
/// 3) 当前目录本身可能就是个任务目录（用户 cd 进去了）→ 用 cwd
pub(crate) fn task_dir(output_root: &Path, id: &str) -> PathBuf {
    let as_path = Path::new(id);
    if as_path.is_dir() {
        return as_path.to_path_buf();
    }
    let under_root = output_root.join(id);
    if under_root.is_dir() {
        return under_root;
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    if cwd.join("rewritten.md").exists() || cwd.join("input.md").exists() {
        return cwd;
    }
    under_root
}

/// 无 --id 时定位任务目录（取 output/ 下最新，或 cwd 本身）
pub(crate) fn latest_task_dir(output_root: &Path) -> anyhow::Result<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_default();
    if cwd.join("rewritten.md").exists() || cwd.join("input.md").exists() {
        return Ok(cwd);
    }
    let mut dirs: Vec<_> = std::fs::read_dir(output_root)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    dirs.sort_by_key(|e| e.file_name());
    dirs.last()
        .map(|e| e.path())
        .ok_or_else(|| anyhow::anyhow!("未找到任务目录，请用 --id 指定，或从项目根目录（含 output/ 的目录）运行"))
}