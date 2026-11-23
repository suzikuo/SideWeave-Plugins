use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TerminalPreference {
    #[default]
    Auto,
    Pebrel,
    WindowsTerminal,
    Cmd,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub(super) struct TerminalSettings {
    pub terminal: TerminalPreference,
    pub pebrel_path: String,
}

pub(super) fn is_executable_path(path: &Path) -> bool {
    path.is_absolute()
        && path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
}

#[cfg(not(windows))]
pub(super) fn open(
    _: &sideweave_plugin_sdk::PluginContext,
    _: &TerminalSettings,
    _: &str,
    _: &[String],
) -> Result<(), String> {
    Err("SSH 终端启动目前仅支持 Windows".into())
}

#[cfg(windows)]
pub(super) use windows::open;

#[cfg(windows)]
mod windows {
    use super::{is_executable_path, TerminalPreference, TerminalSettings};
    use serde::de::{DeserializeOwned, IgnoredAny};
    use serde::Deserialize;
    use sideweave_plugin_sdk::PluginContext;
    use std::io::{BufReader, Read};
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, ExitStatus, Stdio};
    use std::sync::mpsc::{self, Receiver};
    use std::sync::Mutex;
    use std::thread;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::System::Threading::{CREATE_NEW_CONSOLE, CREATE_NO_WINDOW};

    const PEBREL_REPLY_TIMEOUT_MS: &str = "3000";
    const PEBREL_RENAME_TIMEOUT_MS: &str = "1500";
    const PEBREL_PROCESS_TIMEOUT: Duration = Duration::from_secs(5);
    const PEBREL_MAX_TITLE_BYTES: usize = 256;
    static PEBREL_TAB_LAUNCH: Mutex<()> = Mutex::new(());

    pub(crate) fn open(
        context: &PluginContext,
        settings: &TerminalSettings,
        title: &str,
        ssh_args: &[String],
    ) -> Result<(), String> {
        match settings.terminal {
            TerminalPreference::Pebrel => open_pebrel(&settings.pebrel_path, title, ssh_args)
                .map_err(PebrelFailure::into_message),
            TerminalPreference::WindowsTerminal => open_windows_terminal(context, title, ssh_args),
            TerminalPreference::Cmd => open_cmd(ssh_args),
            TerminalPreference::Auto => {
                let pebrel_error = match open_pebrel(&settings.pebrel_path, title, ssh_args) {
                    Ok(()) => return Ok(()),
                    Err(PebrelFailure::NotOpened(message)) => message,
                    Err(PebrelFailure::Unconfirmed(message)) => return Err(message),
                };
                match open_windows_terminal(context, title, ssh_args) {
                    Ok(()) => Ok(()),
                    Err(terminal_error) => open_cmd(ssh_args).map_err(|cmd_error| {
                        format!("{pebrel_error}；{terminal_error}；{cmd_error}")
                    }),
                }
            }
        }
    }

    fn which(name: &str) -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .filter(|directory| directory.is_absolute())
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
    }

    fn system_program(relative: &str) -> Option<PathBuf> {
        let path = PathBuf::from(std::env::var_os("SystemRoot")?).join(relative);
        path.is_file().then_some(path)
    }

    fn open_windows_terminal(
        context: &PluginContext,
        title: &str,
        ssh_args: &[String],
    ) -> Result<(), String> {
        let executable = std::env::var_os("LOCALAPPDATA")
            .map(|local| PathBuf::from(local).join(r"Microsoft\WindowsApps\wt.exe"))
            .filter(|path| path.is_file())
            .or_else(|| which("wt.exe"))
            .ok_or("未找到 Windows Terminal（wt.exe）")?;
        let mut args = vec!["new-tab".into(), "--title".into(), title.into()];
        args.extend_from_slice(ssh_args);
        context
            .system()
            .spawn_with_cwd(&executable.to_string_lossy(), &args, None)
            .map_err(|error| format!("Windows Terminal 启动失败：{}", error.message))
    }

    fn open_cmd(ssh_args: &[String]) -> Result<(), String> {
        let executable =
            system_program(r"System32\cmd.exe").unwrap_or_else(|| PathBuf::from("cmd.exe"));
        let mut command = Command::new(executable);
        command.args(["/D", "/V:OFF", "/S", "/K"]);
        let line = cmd_line(&mut command, ssh_args)?;
        command
            .raw_arg(line)
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("CMD 启动失败：{error}"))
    }

    fn cmd_line(command: &mut Command, args: &[String]) -> Result<String, String> {
        // One expansion inside each quoted argument protects shell characters in profile data.
        let mut references = Vec::with_capacity(args.len());
        for (index, arg) in args.iter().enumerate() {
            if arg.contains(['"', '\0', '\r', '\n']) {
                return Err("SSH 参数包含 CMD 无法传递的字符".into());
            }
            let name = format!("SIDEWEAVE_SSH_ARG_{index}");
            let trailing_slashes = arg.len() - arg.trim_end_matches('\\').len();
            command.env(&name, format!("{arg}{}", "\\".repeat(trailing_slashes)));
            references.push(format!("\"%{name}%\""));
        }
        Ok(format!("\"{}\"", references.join(" ")))
    }

    #[derive(Debug)]
    enum PebrelFailure {
        NotOpened(String),
        Unconfirmed(String),
    }

    impl PebrelFailure {
        fn into_message(self) -> String {
            match self {
                Self::NotOpened(message) | Self::Unconfirmed(message) => message,
            }
        }
    }

    fn find_pebrel(configured: &str) -> Result<PathBuf, PebrelFailure> {
        if !configured.is_empty() {
            let path = Path::new(configured);
            return if is_executable_path(path) && path.is_file() {
                Ok(path.to_owned())
            } else {
                Err(PebrelFailure::NotOpened(format!(
                    "找不到配置的 Pebrel：{configured}"
                )))
            };
        }
        ["PEBREL_CLI", "NEBULA_CLI"]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from)
            .find(|path| is_executable_path(path) && path.is_file())
            .or_else(|| which("pebrel.exe"))
            .ok_or_else(|| {
                PebrelFailure::NotOpened("未找到 Pebrel，请在终端设置中选择 pebrel.exe".into())
            })
    }

    fn open_pebrel(
        configured: &str,
        title: &str,
        ssh_args: &[String],
    ) -> Result<(), PebrelFailure> {
        let executable = find_pebrel(configured)?;
        let ssh = which("ssh.exe")
            .or_else(|| system_program(r"System32\OpenSSH\ssh.exe"))
            .unwrap_or_else(|| PathBuf::from("ssh.exe"));
        // Pebrel renames by window-local index. Keep our own create-and-rename sequences together.
        let _launch = PEBREL_TAB_LAUNCH
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut command = Command::new(&executable);
        command
            .args([
                "tab",
                "new",
                "--timeout-ms",
                PEBREL_REPLY_TIMEOUT_MS,
                "--command",
            ])
            .arg(ssh)
            .args(ssh_args.iter().skip(1));
        let result = run_pebrel::<PebrelTabResult>(&mut command)?;
        let title = pebrel_title(title);
        if let Some((window_id, tab_index)) = result
            .and_then(|result| result.created_tab())
            .filter(|_| !title.is_empty())
        {
            let mut rename = Command::new(executable);
            rename
                .args([
                    "tab",
                    "rename",
                    "--timeout-ms",
                    PEBREL_RENAME_TIMEOUT_MS,
                    "--window",
                ])
                .arg(window_id.to_string())
                .arg("--")
                .arg(tab_index.to_string())
                .arg(title);
            // Renaming can be unavailable on older builds or the tab can close first.
            let _ = run_pebrel::<IgnoredAny>(&mut rename);
        }
        Ok(())
    }

    fn pebrel_title(title: &str) -> String {
        let mut name = String::new();
        for character in title.chars().filter(|character| !character.is_control()) {
            if name.len() + character.len_utf8() > PEBREL_MAX_TITLE_BYTES {
                break;
            }
            name.push(character);
        }
        name
    }

    fn run_pebrel<T: DeserializeOwned + Send + 'static>(
        command: &mut Command,
    ) -> Result<Option<T>, PebrelFailure> {
        let mut child = command
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| PebrelFailure::NotOpened(format!("无法启动 Pebrel：{error}")))?;
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        // Successful replies include a complete workspace snapshot. Decode only the fields we use.
        let reply =
            capture(move || serde_json::from_reader::<_, PebrelReply<T>>(BufReader::new(stdout)));
        let stderr = capture(move || {
            let mut bytes = Vec::new();
            let _ = stderr.take(16 * 1024).read_to_end(&mut bytes);
            bytes
        });
        let deadline = Instant::now() + PEBREL_PROCESS_TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
                result => {
                    if child.kill().is_ok() {
                        let _ = child.wait();
                    }
                    let message = match result {
                        Err(error) => format!("等待 Pebrel 时出错：{error}"),
                        _ => "Pebrel 响应超时，请先检查是否已打开 SSH 标签页".into(),
                    };
                    return Err(PebrelFailure::Unconfirmed(message));
                }
            }
        };
        let reply = receive(reply, deadline)?.ok();
        if let Some(reply) = reply {
            return reply.into_result();
        }
        let stderr = receive(stderr, deadline)?;
        Err(missing_pebrel_reply(
            status,
            &String::from_utf8_lossy(&stderr),
        ))
    }

    fn capture<T: Send + 'static>(read: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(read());
        });
        receiver
    }

    fn receive<T>(receiver: Receiver<T>, deadline: Instant) -> Result<T, PebrelFailure> {
        receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| PebrelFailure::Unconfirmed("无法读取 Pebrel 的连接回执".into()))
    }

    #[derive(Deserialize)]
    struct PebrelReply<T> {
        ok: bool,
        error: Option<PebrelError>,
        result: Option<T>,
    }

    #[derive(Deserialize)]
    struct PebrelTabResult {
        action: Option<PebrelCreatedPane>,
        snapshot: Option<PebrelSnapshot>,
    }

    impl PebrelTabResult {
        fn created_tab(self) -> Option<(u64, usize)> {
            let created = self.action?;
            let window = self
                .snapshot?
                .windows
                .into_iter()
                .find(|window| window.id == created.window_id)?;
            let tab = window
                .tabs
                .into_iter()
                .find(|tab| tab.panes.iter().any(|pane| pane.id == created.pane_id))?;
            Some((created.window_id, tab.index))
        }
    }

    #[derive(Deserialize)]
    struct PebrelCreatedPane {
        window_id: u64,
        pane_id: u64,
    }

    #[derive(Deserialize)]
    struct PebrelSnapshot {
        windows: Vec<PebrelWindow>,
    }

    #[derive(Deserialize)]
    struct PebrelWindow {
        id: u64,
        tabs: Vec<PebrelTab>,
    }

    #[derive(Deserialize)]
    struct PebrelTab {
        index: usize,
        panes: Vec<PebrelPane>,
    }

    #[derive(Deserialize)]
    struct PebrelPane {
        id: u64,
    }

    #[derive(Deserialize)]
    struct PebrelError {
        code: String,
        message: String,
    }

    impl<T> PebrelReply<T> {
        fn into_result(self) -> Result<Option<T>, PebrelFailure> {
            if self.ok {
                return Ok(self.result);
            }
            let Some(error) = self.error else {
                return Err(PebrelFailure::Unconfirmed("Pebrel 未返回失败原因".into()));
            };
            // Fall back only on a rejection that proves no tab was created.
            Err(match error.code.as_str() {
                "runtime_unavailable"
                | "method_not_found"
                | "target_not_found"
                | "protocol_version_mismatch"
                | "invalid_request"
                | "invalid_params" => PebrelFailure::NotOpened(format!(
                    "Pebrel 无法打开 SSH 标签页：{}",
                    error.message
                )),
                _ => PebrelFailure::Unconfirmed(format!(
                    "无法确认 Pebrel 是否已打开 SSH 标签页：{}",
                    error.message
                )),
            })
        }
    }

    fn missing_pebrel_reply(status: ExitStatus, stderr: &str) -> PebrelFailure {
        let Some(detail) = stderr.lines().find(|line| !line.trim().is_empty()) else {
            return PebrelFailure::Unconfirmed(format!("Pebrel 返回了无法识别的回执（{status}）"));
        };
        if detail.starts_with("error: unrecognized subcommand")
            || detail.starts_with("error: unexpected argument")
            || detail.contains("runtime_unavailable")
        {
            PebrelFailure::NotOpened(format!("Pebrel 无法打开 SSH 标签页：{detail}"))
        } else {
            PebrelFailure::Unconfirmed(format!("无法确认 Pebrel 是否已打开 SSH 标签页：{detail}"))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::windows::process::ExitStatusExt;

        fn reply(stdout: &str) -> PebrelReply<IgnoredAny> {
            serde_json::from_reader(stdout.as_bytes()).unwrap()
        }

        #[test]
        fn finds_the_created_tab_in_its_window() {
            let reply: PebrelReply<PebrelTabResult> = serde_json::from_str(
                r#"{
                "ok": true,
                "result": {
                    "action": {"window_id": 2, "pane_id": 7},
                    "snapshot": {"windows": [
                        {"id": 1, "tabs": [{"index": 0, "panes": [{"id": 7}]}]},
                        {"id": 2, "tabs": [
                            {"index": 3, "panes": [{"id": 5}]},
                            {"index": 4, "panes": [{"id": 7}]}
                        ]}
                    ]}
                }
            }"#,
            )
            .unwrap();
            assert_eq!(
                reply
                    .into_result()
                    .unwrap()
                    .and_then(PebrelTabResult::created_tab),
                Some((2, 4))
            );
            let legacy: PebrelReply<PebrelTabResult> =
                serde_json::from_str(r#"{"ok":true,"result":{}}"#).unwrap();
            assert!(legacy
                .into_result()
                .unwrap()
                .and_then(PebrelTabResult::created_tab)
                .is_none());
        }

        #[test]
        fn keeps_unicode_names_within_pebrels_byte_limit() {
            assert_eq!(pebrel_title("-生产\n环境"), "-生产环境");
            assert_eq!(pebrel_title(&"连".repeat(100)), "连".repeat(85));
        }

        #[test]
        fn recognizes_success_and_confirmed_unavailability() {
            let snapshot = format!(
                r#"{{"ok":true,"result":{{"snapshot":"{}"}}}}"#,
                "x".repeat(64 * 1024)
            );
            assert!(reply(&snapshot).into_result().is_ok());
            let unavailable =
                r#"{"ok":false,"error":{"code":"runtime_unavailable","message":"offline"}}"#;
            assert!(matches!(
                reply(unavailable).into_result(),
                Err(PebrelFailure::NotOpened(message)) if message.contains("offline")
            ));
            assert!(matches!(
                missing_pebrel_reply(
                    ExitStatus::from_raw(1),
                    "error: unrecognized subcommand 'tab'"
                ),
                PebrelFailure::NotOpened(_)
            ));
        }

        #[test]
        fn does_not_fall_back_after_an_uncertain_launch() {
            let timeout =
                r#"{"ok":false,"error":{"code":"runtime_timeout","message":"timed out"}}"#;
            assert!(matches!(
                reply(timeout).into_result(),
                Err(PebrelFailure::Unconfirmed(_))
            ));
            assert!(matches!(
                missing_pebrel_reply(ExitStatus::from_raw(1), ""),
                PebrelFailure::Unconfirmed(_)
            ));
        }
    }
}
