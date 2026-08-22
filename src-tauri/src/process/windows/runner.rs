//! Windows PowerShell 5.1 Runner 解析与参数构造。
//!
//! 本文件通过 Windows 系统目录 API 解析绝对可执行文件，不读取可变 `PATH`，并集中维护
//! CmdBox 一次性非交互 PowerShell 任务的固定启动参数。这里只描述调用，不创建进程。

use std::error::Error;
use std::ffi::OsString;
use std::fmt::{Display, Formatter};
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

/// Windows 系统目录 API 的初始缓冲区长度。
const INITIAL_SYSTEM_DIRECTORY_BUFFER: usize = 260;

/// Windows PowerShell 在系统目录下的固定相对位置。
const WINDOWS_POWERSHELL_RELATIVE_PATH: [&str; 3] = ["WindowsPowerShell", "v1.0", "powershell.exe"];

/// Windows PowerShell Runner 解析失败。
#[derive(Debug)]
pub enum RunnerResolveError {
    /// Windows 系统目录 API 调用失败。
    SystemDirectory(io::Error),
    /// 读取固定可执行文件的元数据失败。
    ExecutableMetadata {
        /// 无法读取元数据的固定可执行文件路径。
        path: PathBuf,
        /// 文件系统返回的原始错误。
        source: io::Error,
    },
    /// 固定位置的 Windows PowerShell 可执行文件不存在或不是普通文件。
    ExecutableUnavailable(PathBuf),
}

/// 输出面向开发者的 Runner 解析错误说明。
impl Display for RunnerResolveError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SystemDirectory(source) => {
                write!(formatter, "无法读取 Windows 系统目录：{source}")
            }
            Self::ExecutableMetadata { path, source } => write!(
                formatter,
                "无法读取系统 Windows PowerShell 元数据（{}）：{source}",
                path.display()
            ),
            Self::ExecutableUnavailable(path) => write!(
                formatter,
                "系统 Windows PowerShell 可执行文件不可用：{}",
                path.display()
            ),
        }
    }
}

/// 暴露底层系统错误，便于调用方记录准确失败来源。
impl Error for RunnerResolveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::SystemDirectory(source) => Some(source),
            Self::ExecutableMetadata { source, .. } => Some(source),
            Self::ExecutableUnavailable(_) => None,
        }
    }
}

/// CmdBox 当前支持的确定性 Runner 类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerType {
    /// 系统自带 Windows PowerShell 5.1。
    WindowsPowerShell,
}

impl RunnerType {
    /// 返回供后续 IPC 契约复用的稳定 camelCase 标识。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WindowsPowerShell => "windowsPowershell",
        }
    }
}

/// 系统自带 Windows PowerShell 5.1 的确定性调用描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsPowerShellRunner {
    /// 由 Windows 系统目录推导出的绝对可执行文件路径。
    executable: PathBuf,
}

impl WindowsPowerShellRunner {
    /// 从 Windows 系统目录解析 PowerShell 5.1，不读取 `PATH` 或自动切换到 PowerShell 7。
    pub fn resolve() -> Result<Self, RunnerResolveError> {
        let system_directory = system_directory()?;
        let executable = WINDOWS_POWERSHELL_RELATIVE_PATH
            .iter()
            .fold(system_directory, |path, segment| path.join(segment));

        if !executable.is_absolute() {
            return Err(RunnerResolveError::ExecutableUnavailable(executable));
        }

        let metadata =
            executable
                .metadata()
                .map_err(|source| RunnerResolveError::ExecutableMetadata {
                    path: executable.clone(),
                    source,
                })?;
        if !metadata.is_file() {
            return Err(RunnerResolveError::ExecutableUnavailable(executable));
        }

        Ok(Self { executable })
    }

    /// 返回稳定 Runner 类型，避免上层根据 Rust 具体类型重复推断协议标识。
    pub const fn runner_type(&self) -> RunnerType {
        RunnerType::WindowsPowerShell
    }

    /// 返回确定的 Windows PowerShell 可执行文件绝对路径。
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// 为一个已经生成的 `.ps1` 文件构造固定、非交互且不加载用户 Profile 的参数。
    pub fn script_arguments(&self, script_path: &Path) -> Vec<OsString> {
        vec![
            OsString::from("-NoLogo"),
            OsString::from("-NoProfile"),
            OsString::from("-NonInteractive"),
            OsString::from("-ExecutionPolicy"),
            OsString::from("Bypass"),
            OsString::from("-File"),
            script_path.as_os_str().to_owned(),
        ]
    }
}

/// 使用 Win32 API 读取当前进程对应的系统目录，并处理 API 要求扩展缓冲区的情况。
fn system_directory() -> Result<PathBuf, RunnerResolveError> {
    let mut buffer = vec![0_u16; INITIAL_SYSTEM_DIRECTORY_BUFFER];

    loop {
        // SAFETY: `buffer` 在调用期间保持有效，长度按 u32 传入且远小于 u32 上限；API 只在
        // 给定范围内写入 UTF-16 路径，并以返回值说明实际长度或所需容量。
        let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
        if length == 0 {
            return Err(RunnerResolveError::SystemDirectory(
                io::Error::last_os_error(),
            ));
        }

        let length = length as usize;
        if length < buffer.len() {
            return Ok(PathBuf::from(OsString::from_wide(&buffer[..length])));
        }

        buffer.resize(length + 1, 0);
    }
}

#[cfg(test)]
mod tests {
    //! Windows PowerShell Runner 的真实系统解析与稳定参数测试。

    use std::ffi::{OsStr, OsString};
    use std::path::Path;
    use std::sync::Mutex;

    use super::{RunnerType, WindowsPowerShellRunner};

    /// 串行保护进程级 `PATH` 修改，避免本模块测试彼此污染。
    static PATH_ENVIRONMENT_LOCK: Mutex<()> = Mutex::new(());

    /// 在测试结束或 panic 展开时恢复原始 `PATH`。
    struct PathEnvironmentGuard {
        /// 测试开始前的原始 `PATH`；不存在时保持 `None`。
        original: Option<OsString>,
    }

    impl PathEnvironmentGuard {
        /// 把 `PATH` 临时替换为伪目录，并保存可恢复的原值。
        fn replace(fake_path: &OsStr) -> Self {
            let original = std::env::var_os("PATH");
            std::env::set_var("PATH", fake_path);
            Self { original }
        }
    }

    /// 无论测试如何退出，都恢复进程级 `PATH` 环境变量。
    impl Drop for PathEnvironmentGuard {
        fn drop(&mut self) {
            if let Some(original) = &self.original {
                std::env::set_var("PATH", original);
            } else {
                std::env::remove_var("PATH");
            }
        }
    }

    /// 验证 Runner 始终返回系统中真实存在的绝对 PowerShell 可执行文件。
    #[test]
    fn resolves_existing_absolute_windows_powershell() {
        let runner = WindowsPowerShellRunner::resolve().expect("系统应提供 Windows PowerShell");

        assert!(runner.executable().is_absolute());
        assert!(runner.executable().is_file());
        assert_eq!(
            runner
                .executable()
                .file_name()
                .and_then(|name| name.to_str()),
            Some("powershell.exe")
        );
        assert_eq!(runner.runner_type(), RunnerType::WindowsPowerShell);
        assert_eq!(runner.runner_type().as_str(), "windowsPowershell");
    }

    /// 验证伪造 `PATH` 不会改变 Runner 的系统目录解析结果。
    #[test]
    fn ignores_path_when_resolving_windows_powershell() {
        let _lock = PATH_ENVIRONMENT_LOCK
            .lock()
            .expect("PATH 测试互斥锁不应中毒");
        let baseline = WindowsPowerShellRunner::resolve().expect("系统应提供 Windows PowerShell");
        let fake_path = OsStr::new(r"Z:\CmdBox\fake-path-containing-powershell");
        let _environment = PathEnvironmentGuard::replace(fake_path);

        let resolved = WindowsPowerShellRunner::resolve().expect("解析不应依赖 PATH");

        assert_eq!(resolved.executable(), baseline.executable());
        assert!(!resolved.executable().starts_with(fake_path));
    }

    /// 验证固定非交互参数的顺序和 `-File` 后脚本路径不会发生漂移。
    #[test]
    fn builds_stable_non_interactive_script_arguments() {
        let runner = WindowsPowerShellRunner::resolve().expect("系统应提供 Windows PowerShell");
        let script_path = Path::new(r"C:\Temp\CmdBox\execution-id\script.ps1");

        assert_eq!(
            runner.script_arguments(script_path),
            vec![
                OsString::from("-NoLogo"),
                OsString::from("-NoProfile"),
                OsString::from("-NonInteractive"),
                OsString::from("-ExecutionPolicy"),
                OsString::from("Bypass"),
                OsString::from("-File"),
                script_path.as_os_str().to_owned(),
            ]
        );
    }
}
