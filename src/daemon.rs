//! serve 子命令的后台进程管理：start / stop / restart / status
//!
//! 后台模式通过「重 exec 自身 + setsid」实现：
//! 父进程 fork 出子进程（`serve --foreground`），子进程调用 setsid 脱离
//! 终端会话，避免 SSH 断开时被 SIGHUP 杀掉；日志追加到
//! ~/.media-factory/logs/serve.log，PID 写入 ~/.media-factory/serve.pid。

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

fn state_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".media-factory")
}

fn pid_file() -> PathBuf {
    state_dir().join("serve.pid")
}

fn log_file() -> PathBuf {
    state_dir().join("logs").join("serve.log")
}

/// 读取 PID 文件并校验进程是否存活；失效的 PID 文件会被清理
fn running_pid() -> Option<u32> {
    let raw = fs::read_to_string(pid_file()).ok()?;
    let pid: u32 = raw.trim().parse().ok()?;
    if pid_alive(pid) {
        Some(pid)
    } else {
        let _ = fs::remove_file(pid_file());
        None
    }
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    // kill(pid, 0)：不发信号，仅探测进程是否存在
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
fn pid_alive(_pid: u32) -> bool {
    false
}

fn wait_port_ready(port: u16, tries: u32) -> bool {
    for _ in 0..tries {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    false
}

/// 后台启动服务；已在运行则直接提示
#[cfg(unix)]
pub fn start(port: u16) -> Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    if let Some(pid) = running_pid() {
        println!("✓ media-factory 已在后台运行 (PID {pid})，地址 http://localhost:{port}");
        return Ok(());
    }

    let log_path = log_file();
    fs::create_dir_all(log_path.parent().unwrap())?;
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("无法打开日志文件 {}", log_path.display()))?;
    let log_err = log.try_clone()?;

    let exe = std::env::current_exe().context("无法定位当前可执行文件")?;
    let mut cmd = Command::new(exe);
    cmd.arg("serve")
        .arg("--port")
        .arg(port.to_string())
        .arg("--foreground")
        .current_dir(state_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));

    // 脱离终端会话，SSH 断开后进程继续存活
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd.spawn().context("后台进程启动失败")?;
    fs::write(pid_file(), child.id().to_string())?;

    // 等待端口就绪；期间子进程若提前退出（如端口被占用）则报错
    for _ in 0..40 {
        if let Some(status) = child.try_wait()? {
            let _ = fs::remove_file(pid_file());
            anyhow::bail!(
                "服务启动即退出（{status}），请查看日志: {}",
                log_path.display()
            );
        }
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            println!("✓ 已在后台启动 (PID {})", child.id());
            println!("  地址: http://localhost:{port}（外网访问请用 http://<服务器IP>:{port}）");
            println!("  日志: {}", log_path.display());
            println!("  工作目录: {}（任务产物 output/ 在此目录下）", state_dir().display());
            println!("  停止: media-factory serve --stop");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    println!("⚠ 进程已启动 (PID {})，但端口 {port} 探测未通过，请查看日志: {}", child.id(), log_path.display());
    Ok(())
}

#[cfg(not(unix))]
pub fn start(_port: u16) -> Result<()> {
    anyhow::bail!("当前平台不支持后台运行，请使用 media-factory serve --foreground 前台运行")
}

/// 停止后台服务
#[cfg(unix)]
pub fn stop() -> Result<()> {
    let Some(pid) = running_pid() else {
        println!("○ 服务未在运行");
        return Ok(());
    };

    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
    for _ in 0..20 {
        if !pid_alive(pid) {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    if pid_alive(pid) {
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
    let _ = fs::remove_file(pid_file());
    println!("✓ 已停止 (PID {pid})");
    Ok(())
}

#[cfg(not(unix))]
pub fn stop() -> Result<()> {
    anyhow::bail!("当前平台不支持后台进程管理")
}

/// 查看服务状态
pub fn status(port: u16) -> Result<()> {
    match running_pid() {
        Some(pid) => println!("● 运行中 (PID {pid})，端口 {port} → http://localhost:{port}"),
        None => println!("○ 未运行"),
    }
    Ok(())
}

/// 供 start 失败场景使用：端口就绪探测（供测试）
#[allow(dead_code)]
pub fn probe(port: u16) -> bool {
    wait_port_ready(port, 1)
}
