use super::config_manager::config_yaml_path;
use super::models::TunnelState;
use chrono::Local;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
#[cfg(windows)]
use std::process::Output;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const MAX_LIVE_LOG_LINES: usize = 50;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

struct SingleProcessState {
    child: Arc<Mutex<Option<Child>>>,
    conns: u32,
    error_message: Option<String>,
    logs: Arc<Mutex<VecDeque<String>>>,
}

static STATE: OnceLock<Mutex<SingleProcessState>> = OnceLock::new();
static LIFECYCLE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn process_state() -> &'static Mutex<SingleProcessState> {
    STATE.get_or_init(|| {
        Mutex::new(SingleProcessState {
            child: Arc::new(Mutex::new(None)),
            conns: 0,
            error_message: None,
            logs: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LIVE_LOG_LINES))),
        })
    })
}
fn lifecycle_lock() -> &'static Mutex<()> {
    LIFECYCLE_LOCK.get_or_init(|| Mutex::new(()))
}

/// 检查 Windows 系统中是否有 cloudflared.exe 正在运行
pub fn is_cloudflared_running_globally() -> bool {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("tasklist");
        cmd.args(["/FI", "IMAGENAME eq cloudflared.exe", "/NH"])
            .creation_flags(CREATE_NO_WINDOW);
        if let Ok(output) = cmd.output() {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout).to_lowercase();
                return text.contains("cloudflared.exe");
            }
        }

        // tasklist can fail to enumerate processes in restricted host contexts.
        // Get-Process is a useful fallback and reports only whether a process exists.
        let mut cmd = Command::new("powershell.exe");
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "if (Get-Process -Name cloudflared -ErrorAction SilentlyContinue) { exit 0 } else { exit 1 }",
        ])
        .creation_flags(CREATE_NO_WINDOW);
        if let Ok(output) = cmd.output() {
            return output.status.success();
        }
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("pgrep");
        cmd.arg("cloudflared");
        if let Ok(output) = cmd.output() {
            return output.status.success();
        }
    }
    false
}

#[cfg(windows)]
fn run_hidden_command(program: &str, args: &[&str]) -> std::io::Result<Output> {
    Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
}

#[cfg(windows)]
fn is_access_denied(output: &Output) -> bool {
    if output.status.code() == Some(5) {
        return true;
    }
    let text = format!(
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_lowercase();
    text.contains("failed 5") || text.contains("access is denied")
}

#[cfg(windows)]
fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(windows)]
fn run_elevated_command(program: &str, args: &[&str]) -> Result<Output, String> {
    let quoted_args = args
        .iter()
        .map(|arg| powershell_quote(arg))
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        "$p = Start-Process -FilePath {} -ArgumentList @({}) -Verb RunAs -Wait -PassThru; exit $p.ExitCode",
        powershell_quote(program),
        quoted_args
    );
    run_hidden_command(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
    )
    .map_err(|e| format!("无法请求管理员权限来管理 cloudflared：{e}"))
}

#[cfg(windows)]
fn run_command_with_elevation(program: &str, args: &[&str]) -> Result<Output, String> {
    let output = run_hidden_command(program, args)
        .map_err(|e| format!("无法运行 cloudflared 管理命令：{e}"))?;
    if output.status.success() || !is_access_denied(&output) {
        return Ok(output);
    }

    run_elevated_command(program, args)
}

#[cfg(windows)]
fn windows_service_is_stopped() -> Result<Option<bool>, String> {
    let output = run_hidden_command("sc.exe", &["query", "Cloudflared"])
        .map_err(|e| format!("无法查询 Cloudflared 服务：{e}"))?;
    let text = format!(
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_uppercase();

    // ERROR_SERVICE_DOES_NOT_EXIST
    if text.contains("1060") {
        return Ok(None);
    }
    if !output.status.success() {
        return Err("无法查询 Windows Cloudflared 服务状态".to_string());
    }
    if let Some(state_code) = text
        .lines()
        .find(|line| line.contains("STATE"))
        .and_then(|line| line.split(':').nth(1))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse::<u32>().ok())
    {
        // SERVICE_STOPPED is 1; every other state may still own a process.
        return Ok(Some(state_code == 1));
    }
    if text.contains("STOPPED") {
        return Ok(Some(true));
    }
    Ok(Some(false))
}

#[cfg(windows)]
fn stop_cloudflared_service() -> Result<(), String> {
    if windows_service_is_stopped()? != Some(false) {
        return Ok(());
    }

    let output = run_command_with_elevation("sc.exe", &["stop", "Cloudflared"])?;
    if !output.status.success() {
        // It may have stopped between the query and the stop request.
        if windows_service_is_stopped()? != Some(false) {
            return Ok(());
        }
        return Err(
            "无法停止 Windows Cloudflared 服务。请在管理员提示中允许操作；若提示已取消，请重试。"
                .to_string(),
        );
    }

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if windows_service_is_stopped()? != Some(false) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("Windows Cloudflared 服务在 15 秒内未能停止".to_string());
        }
        thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(windows)]
fn kill_remaining_cloudflared() -> Result<(), String> {
    if !is_cloudflared_running_globally() {
        return Ok(());
    }

    let output = run_hidden_command("taskkill.exe", &["/F", "/T", "/IM", "cloudflared.exe"])
        .map_err(|e| format!("无法运行 cloudflared 清理命令：{e}"))?;
    if output.status.success() && !is_cloudflared_running_globally() {
        return Ok(());
    }

    // taskkill emits localized text and may use a non-access-denied exit code
    // for protected processes, so retry elevated whenever a process remains.
    let elevated = run_elevated_command("taskkill.exe", &["/F", "/T", "/IM", "cloudflared.exe"])?;
    if !elevated.status.success() && is_cloudflared_running_globally() {
        return Err(
            "无法结束剩余的 cloudflared.exe 进程。请在管理员提示中允许操作；若提示已取消，请重试。"
                .to_string(),
        );
    }
    Ok(())
}

/// 彻底清理所有 cloudflared 进程，并确认退出后才允许下一次启动。
pub fn kill_all_cloudflared() -> Result<(), String> {
    if let Ok(mut state) = process_state().lock() {
        if let Ok(mut c_guard) = state.child.lock() {
            if let Some(mut child) = c_guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        state.conns = 0;
    }

    #[cfg(windows)]
    {
        // SCM-owned processes must be stopped through the service controller first.
        // taskkill alone cannot reliably stop a service and the service may relaunch it.
        stop_cloudflared_service()?;
        kill_remaining_cloudflared()?;
    }
    #[cfg(not(windows))]
    {
        let output = Command::new("pkill")
            .args(["-9", "cloudflared"])
            .output()
            .map_err(|e| format!("停止 cloudflared 失败: {e}"))?;
        if !output.status.success() && is_cloudflared_running_globally() {
            return Err("停止 cloudflared 失败".to_string());
        }
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    while is_cloudflared_running_globally() {
        if Instant::now() >= deadline {
            return Err("cloudflared 未能在 5 秒内完全退出".to_string());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

pub fn get_tunnel_status() -> (TunnelState, u32, Option<String>) {
    // 优先检查由本插件管理的子进程状态
    if let Ok(state) = process_state().lock() {
        if let Ok(mut c_guard) = state.child.lock() {
            if let Some(child) = c_guard.as_mut() {
                match child.try_wait() {
                    Ok(None) => {
                        return (
                            TunnelState::Running,
                            state.conns.max(1),
                            state.error_message.clone(),
                        );
                    }
                    Ok(Some(status)) => {
                        *c_guard = None;
                        if !status.success() {
                            return (TunnelState::Error, 0, state.error_message.clone());
                        }
                    }
                    Err(_) => {
                        *c_guard = None;
                    }
                }
            }
        }
    }

    let running = is_cloudflared_running_globally();
    if !running {
        return (TunnelState::Stopped, 0, None);
    }

    if let Ok(state) = process_state().lock() {
        return (
            TunnelState::Running,
            state.conns.max(1),
            state.error_message.clone(),
        );
    }

    (TunnelState::Running, 1, None)
}

pub fn get_tunnel_logs(limit: Option<usize>) -> Vec<String> {
    if let Ok(state) = process_state().lock() {
        if let Ok(logs) = state.logs.lock() {
            let limit = limit.unwrap_or(MAX_LIVE_LOG_LINES);
            let start = logs.len().saturating_sub(limit);
            return logs.iter().skip(start).cloned().collect();
        }
    }
    Vec::new()
}

pub fn clear_tunnel_logs() -> Result<(), String> {
    if let Ok(state) = process_state().lock() {
        if let Ok(mut logs) = state.logs.lock() {
            logs.clear();
        }
    }
    Ok(())
}

fn append_live_log(logs: &Arc<Mutex<VecDeque<String>>>, line: &str) {
    let now = Local::now().format("%H:%M:%S");
    let formatted = format!("[{now}] {line}");

    if let Ok(mut log_guard) = logs.lock() {
        if log_guard.len() >= MAX_LIVE_LOG_LINES {
            log_guard.pop_front();
        }
        log_guard.push_back(formatted);
    }
}

pub fn start_tunnel() -> Result<(), String> {
    let _lifecycle_guard = lifecycle_lock()
        .lock()
        .map_err(|_| "进程生命周期锁破坏".to_string())?;
    let cfg_path = config_yaml_path();
    if !cfg_path.exists() {
        return Err(format!("找不到配置文件: {}", cfg_path.display()));
    }

    // 1. 彻底杀掉已有的 cloudflared 进程，确保绝不产生重复孤儿进程
    kill_all_cloudflared()?;

    let cfg_str = cfg_path.to_string_lossy().into_owned();

    let mut cmd = Command::new("cloudflared");
    cmd.arg("tunnel")
        .arg("--config")
        .arg(&cfg_str)
        .arg("run")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动 cloudflared 失败（请确认已安装并在 PATH 中）: {e}"))?;

    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child_arc = Arc::new(Mutex::new(Some(child)));

    let logs_arc;
    {
        let mut state = process_state()
            .lock()
            .map_err(|_| "进程状态锁破坏".to_string())?;

        state.child = child_arc.clone();
        state.conns = 0;
        state.error_message = None;
        logs_arc = state.logs.clone();
    }

    append_live_log(&logs_arc, &format!("已启动 cloudflared (PID: {pid})"));

    if let Some(out) = stdout {
        let l_arc = logs_arc.clone();
        thread::Builder::new()
            .name(format!("cf-stdout-{pid}"))
            .spawn(move || {
                let reader = BufReader::new(out);
                for line in reader.lines().flatten() {
                    process_log_event(&line);
                    append_live_log(&l_arc, &line);
                }
            })
            .ok();
    }

    if let Some(err) = stderr {
        let l_arc = logs_arc.clone();
        thread::Builder::new()
            .name(format!("cf-stderr-{pid}"))
            .spawn(move || {
                let reader = BufReader::new(err);
                for line in reader.lines().flatten() {
                    process_log_event(&line);
                    append_live_log(&l_arc, &line);
                }
            })
            .ok();
    }

    let child_monitor = child_arc;
    thread::Builder::new()
        .name(format!("cf-monitor-{pid}"))
        .spawn(move || {
            let mut waiter = None;
            if let Ok(mut guard) = child_monitor.lock() {
                waiter = guard.take();
            }
            if let Some(mut c) = waiter {
                let exit_status = c.wait();
                if let Ok(mut state) = process_state().lock() {
                    if let Err(e) = exit_status {
                        state.error_message = Some(format!("进程退出异常: {e}"));
                    }
                }
            }
        })
        .ok();

    Ok(())
}

fn process_log_event(line: &str) {
    let lower = line.to_lowercase();
    if lower.contains("registered tunnel connection")
        || lower.contains("connection registered")
        || lower.contains("connected to")
    {
        if let Ok(mut state) = process_state().lock() {
            state.conns = state.conns.saturating_add(1);
        }
    } else if lower.contains("failed to connect") || lower.contains("tunnel cannot be reached") {
        if let Ok(mut state) = process_state().lock() {
            state.error_message = Some(line.to_string());
        }
    }
}

pub fn stop_tunnel() -> Result<(), String> {
    let _lifecycle_guard = lifecycle_lock()
        .lock()
        .map_err(|_| "进程生命周期锁破坏".to_string())?;
    kill_all_cloudflared()
}

pub fn restart_tunnel() -> Result<(), String> {
    // start_tunnel 已在同一把生命周期锁内先结束所有旧进程，再启动新实例。
    start_tunnel()
}
