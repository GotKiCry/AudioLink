//! 原子写 —— 架构 §9 的明文要求：「写入一律**原子写**（临时文件 + rename），避免断电产生半截配置」。
//!
//! 顺序刻意固定为「写临时文件 → `sync_all` → 关句柄 → rename」：
//!
//! 1. 临时文件与目标**同目录**（rename 只在同一卷内原子，跨卷会退化成复制）；
//! 2. 先 `sync_all` 再改名：保证改名后读到的文件内容已经落盘，不会出现「文件在、内容空」；
//! 3. 关句柄再改名：Windows 上目标/源文件被占用（未共享删除）会让 rename 直接失败；
//! 4. 任何失败都要清掉临时文件，不给下一次启动留垃圾。
//!
//! 已知边界（不在本轮范围）：目录项本身的持久化（POSIX 需 `fsync` 目录）没有对应可移植操作，
//! Windows 目标上更是没有等价物 —— 本函数保证的是「读者永远看到完整的旧版或完整的新版」。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use rand::Rng;

use crate::error::IdentityError;

/// 目标文件的父目录（空串 = 相对当前目录，按原样使用）。
fn parent_dir(path: &Path) -> Result<PathBuf, IdentityError> {
    match path.parent() {
        Some(dir) => Ok(dir.to_path_buf()),
        None => Err(IdentityError::invalid_config_owned(format!(
            "路径没有父目录，无法原子写：{}",
            path.display()
        ))),
    }
}

/// 原子地把 `contents` 写到 `path`：先写同目录临时文件，再 rename 覆盖。
///
/// 权限：临时文件在 Unix 上以 `0600` 创建（证书与私钥都是用户私有数据）；
/// Windows 侧不额外设 ACL，依赖 `%APPDATA%` / 应用私有目录本身的用户级 ACL
/// （架构 §9 提到的 DPAPI 保护属 M2）。
pub(crate) fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), IdentityError> {
    let dir = parent_dir(path)?;
    if !dir.as_os_str().is_empty() {
        // 首次运行时配置目录可能还不存在；建目录失败要显式报错，不能让调用方以为写成功了。
        fs::create_dir_all(&dir).map_err(|error| {
            IdentityError::io_owned(format!("创建目录 {} 失败：{error}", dir.display()))
        })?;
    }

    let file_name = path
        .file_name()
        .ok_or_else(|| {
            IdentityError::invalid_config_owned(format!("路径不是文件名结尾：{}", path.display()))
        })?
        .to_string_lossy()
        .into_owned();

    // 唯一临时名：目标名 + 16 位随机 hex。同名碰撞（或上次崩溃留下的残骸）不会导致误覆盖，
    // 因为下面用 `create_new(true)` 打开 —— 已存在就直接失败。
    let mut temp_path = dir.clone();
    temp_path.push(format!(
        ".{file_name}.{:016x}.tmp",
        rand::rng().random::<u64>()
    ));

    let result = write_temp_then_rename(&temp_path, path, contents);
    if result.is_err() {
        // 失败擦干净：临时文件里的内容可能包含私钥，不能留在磁盘上。
        let _ = fs::remove_file(&temp_path);
    }
    result.map_err(|error| {
        IdentityError::io_owned(format!("原子写 {} 失败：{error}", path.display()))
    })
}

/// 真正干活的部分：返回原始 `io::Error`，便于上层套上路径上下文。
fn write_temp_then_rename(temp_path: &Path, target: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // 临时文件是私钥/证书的载体：从创建那一刻起就只有本用户可读写。
        options.mode(0o600);
    }

    let mut file = options.open(temp_path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    // 先关句柄：Windows 上 rename 需要源文件可被移动（见文件头第 3 条）。
    drop(file);

    // 同目录 rename：POSIX 是 rename(2)，Windows 是 MoveFileExW(REPLACE_EXISTING) —— 两者都在
    // 「同名替换」这一语义下原子。
    fs::rename(temp_path, target)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn 首次写入与覆盖写入都成功() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cert.pem");

        write_atomic(&path, b"v1").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"v1");

        // 覆盖已存在的文件（Windows 上走 MOVEFILE_REPLACE_EXISTING）必须成功。
        write_atomic(&path, b"v2-longer").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"v2-longer");
    }

    #[test]
    fn 目录不存在时会自动创建() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity").join("cert.pem");

        write_atomic(&path, b"pem").unwrap();
        assert!(path.exists());
    }

    #[test]
    fn 不留下临时文件() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cert.pem");
        write_atomic(&path, b"pem").unwrap();

        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "残留临时文件：{leftovers:?}");
    }

    #[test]
    fn invalid_target_path_errors_instead_of_panicking() {
        // Windows 上 `C:` 这类驱动器相对路径的父目录为空且无法建目录；不 panic 即可。
        assert!(
            write_atomic(Path::new(""), b"x").is_err()
                || write_atomic(Path::new(".."), b"x").is_err()
        );
    }
}
