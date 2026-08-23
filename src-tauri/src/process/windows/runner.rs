//! Windows PowerShell 5.1 与 CMD Runner 解析及字段私有的进程启动值构造。
//!
//! 本文件通过 Windows 系统目录 API 解析绝对可执行文件，不读取可变 `PATH`，并集中维护
//! CmdBox 一次性非交互任务的固定启动参数。CMD 额外确定解析 `chcp.com`，并以固定 raw
//! command tail 和启动专用环境变量承载随机 Artifact 路径；任何用户值都不会进入命令行。

use std::collections::BTreeMap;
use std::error::Error;
use std::ffi::OsString;
use std::fmt::{Display, Formatter};
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

use crate::execution::artifact::{ArtifactError, MaterializedScript};

/// Windows 系统目录 API 的初始缓冲区长度。
const INITIAL_SYSTEM_DIRECTORY_BUFFER: usize = 260;

/// Windows PowerShell 在系统目录下的固定相对位置。
const WINDOWS_POWERSHELL_RELATIVE_PATH: [&str; 3] = ["WindowsPowerShell", "v1.0", "powershell.exe"];

/// CMD 在 Windows 系统目录中的固定文件名。
const CMD_EXECUTABLE_FILE_NAME: &str = "cmd.exe";

/// 切换 CMD Code Page 的系统工具固定文件名。
const CHCP_EXECUTABLE_FILE_NAME: &str = "chcp.com";

/// CMD 启动时绑定随机 `.cmd` 路径的内部保留环境变量名。
pub(crate) const CMD_ARTIFACT_ENVIRONMENT_NAME: &str = "CMDBOX_INTERNAL_ARTIFACT";

/// CMD Batch ASCII 前导绑定确定 `chcp.com` 路径的内部保留环境变量名。
pub(crate) const CMD_CHCP_ENVIRONMENT_NAME: &str = "CMDBOX_INTERNAL_CHCP";

/// CMD 分派 Batch 时必需、由系统目录确定推导并进入 Hash 的 Windows 根环境名。
pub(crate) const CMD_SYSTEM_ROOT_ENVIRONMENT_NAME: &str = "SystemRoot";

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
    /// 系统目录无法确定推导出有效 Windows 根目录。
    SystemRootUnavailable(PathBuf),
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
                "无法读取系统 Runner 元数据（{}）：{source}",
                path.display()
            ),
            Self::ExecutableUnavailable(path) => write!(
                formatter,
                "系统 Runner 可执行文件不可用：{}",
                path.display()
            ),
            Self::SystemRootUnavailable(path) => {
                write!(
                    formatter,
                    "无法从系统目录确定 Windows 根目录：{}",
                    path.display()
                )
            }
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
            Self::SystemRootUnavailable(_) => None,
        }
    }
}

/// CmdBox 当前支持的确定性 Runner 类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerType {
    /// 系统自带 Windows PowerShell 5.1。
    WindowsPowerShell,
    /// 系统自带 CMD。
    Cmd,
}

impl RunnerType {
    /// 返回供后续 IPC 契约复用的稳定 camelCase 标识。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WindowsPowerShell => "windowsPowerShell",
            Self::Cmd => "cmd",
        }
    }
}

/// 系统自带 Windows PowerShell 5.1 的确定性解析入口。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsPowerShellRunner;

/// 系统自带 CMD 与同目录 `chcp.com` 的确定性解析入口。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CmdRunner;

/// CreateProcessW 对当前 Runner 使用的环境来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProcessLaunchEnvironment {
    /// 保持 PowerShell 既有行为，由 Windows 继承父进程环境。
    Inherit,
    /// CMD 使用完全替换环境，避免未进 Hash 的父环境影响执行语义。
    Replace(BTreeMap<String, OsString>),
}

/// 临时脚本路径进入 Runner 的唯一受控方式。
#[derive(Debug, Clone, PartialEq, Eq)]
enum ScriptPathBinding {
    /// PowerShell 通过标准 Windows argv quoting 接收 `-File` 后的路径参数。
    Argument,
    /// CMD 通过启动专用环境和固定 raw `/C` tail 展开随机 Artifact 路径。
    Environment(&'static str),
}

/// 已确定可执行文件与固定参数、尚未绑定临时脚本路径的 Runner。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRunner {
    /// 不随环境或 PATH 改变的 Runner 类型。
    runner_type: RunnerType,
    /// 由 Windows 系统目录推导出的绝对可执行文件路径。
    executable: PathBuf,
    /// 不含动态脚本路径的固定参数，顺序属于 Runner 执行语义。
    fixed_arguments: Vec<OsString>,
    /// CMD `/C` 使用的固定 raw tail；PowerShell 没有该字段。
    raw_command_tail: Option<OsString>,
    /// 当前 Runner 绑定动态 Artifact 路径的受控方式。
    script_path_binding: ScriptPathBinding,
    /// Runner 自身必须注入且进入 Canonical Spec 的确定环境，例如绝对 `chcp.com`。
    fixed_environment: BTreeMap<String, OsString>,
    /// 当前 Runner 的环境继承策略。
    environment_replacement: bool,
}

/// 字段私有、已绑定受管脚本租约与完整 Win32 启动参数的进程启动值。
#[derive(Debug)]
pub struct ProcessLaunch {
    /// CreateProcessW 使用的确定性绝对可执行文件路径。
    executable: PathBuf,
    /// 包含固定 Runner 选项与受管脚本路径的完整 argv。
    arguments: Vec<OsString>,
    /// CMD `/C` 之后无需 CRT quoting 的固定 raw command tail。
    raw_command_tail: Option<OsString>,
    /// 由上层 Execution 选择、仍需进程内核验证的工作目录。
    working_directory: PathBuf,
    /// PowerShell 继承父环境；CMD 只使用完整确定的替换环境。
    environment: ProcessLaunchEnvironment,
    /// 保持临时脚本及其唯一目录到受管进程生命周期结束的 RAII 租约。
    materialized_script: MaterializedScript,
}

impl WindowsPowerShellRunner {
    /// 从 Windows 系统目录解析 PowerShell 5.1，不读取 `PATH` 或自动切换到 PowerShell 7。
    pub fn resolve() -> Result<ResolvedRunner, RunnerResolveError> {
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

        Ok(ResolvedRunner {
            runner_type: RunnerType::WindowsPowerShell,
            executable,
            fixed_arguments: vec![
                OsString::from("-NoLogo"),
                OsString::from("-NoProfile"),
                OsString::from("-NonInteractive"),
                OsString::from("-ExecutionPolicy"),
                OsString::from("Bypass"),
                OsString::from("-File"),
            ],
            raw_command_tail: None,
            script_path_binding: ScriptPathBinding::Argument,
            fixed_environment: BTreeMap::new(),
            environment_replacement: false,
        })
    }
}

impl CmdRunner {
    /// 从 Windows 系统目录解析 `cmd.exe` 与 `chcp.com`，不读取 `PATH` 或 ComSpec。
    pub fn resolve() -> Result<ResolvedRunner, RunnerResolveError> {
        let system_directory = system_directory()?;
        let system_root = system_directory
            .parent()
            .filter(|path| path.is_absolute() && path.is_dir())
            .ok_or_else(|| RunnerResolveError::SystemRootUnavailable(system_directory.clone()))?
            .to_path_buf();
        let executable = system_directory.join(CMD_EXECUTABLE_FILE_NAME);
        ensure_regular_file(&executable)?;
        let chcp = system_directory.join(CHCP_EXECUTABLE_FILE_NAME);
        ensure_regular_file(&chcp)?;

        let mut fixed_environment = BTreeMap::new();
        fixed_environment.insert(
            CMD_CHCP_ENVIRONMENT_NAME.to_owned(),
            chcp.as_os_str().to_owned(),
        );
        fixed_environment.insert(
            CMD_SYSTEM_ROOT_ENVIRONMENT_NAME.to_owned(),
            system_root.as_os_str().to_owned(),
        );
        Ok(ResolvedRunner {
            runner_type: RunnerType::Cmd,
            executable,
            fixed_arguments: vec![
                OsString::from("/D"),
                OsString::from("/Q"),
                OsString::from("/A"),
                OsString::from("/E:ON"),
                OsString::from("/V:ON"),
                OsString::from("/S"),
                OsString::from("/C"),
            ],
            raw_command_tail: Some(OsString::from(format!(
                "\"\"!{CMD_ARTIFACT_ENVIRONMENT_NAME}!\"\""
            ))),
            script_path_binding: ScriptPathBinding::Environment(CMD_ARTIFACT_ENVIRONMENT_NAME),
            fixed_environment,
            environment_replacement: true,
        })
    }
}

impl ResolvedRunner {
    /// 返回稳定 Runner 类型，避免上层根据 Rust 具体类型重复推断协议标识。
    pub const fn runner_type(&self) -> RunnerType {
        self.runner_type
    }

    /// 返回确定的 Windows PowerShell 可执行文件绝对路径。
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// 返回动态脚本路径之前的固定 Runner 选项，供 Canonical Execution Spec 绑定执行语义。
    pub(crate) fn fixed_arguments(&self) -> &[OsString] {
        &self.fixed_arguments
    }

    /// 返回固定 raw command tail；该值属于 CMD 执行语义并必须进入 Canonical Spec。
    pub(crate) fn raw_command_tail(&self) -> Option<&OsString> {
        self.raw_command_tail.as_ref()
    }

    /// 返回 Runner 自身需要且必须进入 Canonical Spec 的确定内部环境。
    pub(crate) fn fixed_environment(&self) -> &BTreeMap<String, OsString> {
        &self.fixed_environment
    }

    /// 绑定一个受管临时脚本与工作目录，生成字段私有的完整进程启动值。
    pub fn process_launch(
        self,
        materialized_script: MaterializedScript,
        working_directory: &Path,
    ) -> ProcessLaunch {
        self.process_launch_with_environment(
            materialized_script,
            working_directory,
            BTreeMap::new(),
        )
    }

    /// 绑定已经进入 Canonical Spec 的环境覆盖与受管脚本，生成不可变启动值。
    pub(crate) fn process_launch_with_environment(
        self,
        materialized_script: MaterializedScript,
        working_directory: &Path,
        mut environment_overrides: BTreeMap<String, OsString>,
    ) -> ProcessLaunch {
        for (name, value) in &self.fixed_environment {
            environment_overrides.insert(name.clone(), value.clone());
        }
        let arguments = match self.script_path_binding {
            ScriptPathBinding::Argument => {
                let mut arguments = self.fixed_arguments.clone();
                arguments.push(materialized_script.script_path().as_os_str().to_owned());
                arguments
            }
            ScriptPathBinding::Environment(name) => {
                environment_overrides.insert(
                    name.to_owned(),
                    materialized_script.script_path().as_os_str().to_owned(),
                );
                self.fixed_arguments.clone()
            }
        };
        let environment = if self.environment_replacement {
            ProcessLaunchEnvironment::Replace(environment_overrides)
        } else {
            debug_assert!(
                environment_overrides.is_empty(),
                "PowerShell 基线不得通过替换环境改变继承语义"
            );
            ProcessLaunchEnvironment::Inherit
        };
        ProcessLaunch {
            executable: self.executable,
            arguments,
            raw_command_tail: self.raw_command_tail,
            working_directory: working_directory.to_path_buf(),
            environment,
            materialized_script,
        }
    }
}

impl ProcessLaunch {
    /// 返回当前受管 Artifact 的唯一临时目录，仅供 Session 清理回归测试观察。
    #[cfg(test)]
    pub(crate) fn temporary_directory(&self) -> &Path {
        self.materialized_script
            .script_path()
            .parent()
            .expect("受管脚本必须位于唯一临时目录")
    }

    /// 返回 CreateProcessW 使用的确定性绝对可执行文件路径。
    pub(crate) fn executable(&self) -> &Path {
        &self.executable
    }

    /// 返回已经包含受管脚本路径的完整 Runner 参数。
    pub(crate) fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    /// 返回 CMD 专用固定 raw command tail；PowerShell 为 `None`。
    pub(crate) fn raw_command_tail(&self) -> Option<&OsString> {
        self.raw_command_tail.as_ref()
    }

    /// 返回调用方选择且由进程内核再次验证的工作目录。
    pub(crate) fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    /// 返回 CreateProcessW 使用的继承或完全替换环境模式。
    pub(crate) fn environment(&self) -> &ProcessLaunchEnvironment {
        &self.environment
    }

    /// 紧邻 CreateProcessW 前复验当前启动值持有的脚本字节 Hash。
    pub(crate) fn verify_before_spawn(&self) -> Result<(), ArtifactError> {
        self.materialized_script.verify_before_spawn()
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

/// 确认从系统目录推导出的 Runner 或辅助工具路径是绝对普通文件。
fn ensure_regular_file(path: &Path) -> Result<(), RunnerResolveError> {
    if !path.is_absolute() {
        return Err(RunnerResolveError::ExecutableUnavailable(
            path.to_path_buf(),
        ));
    }
    let metadata = path
        .metadata()
        .map_err(|source| RunnerResolveError::ExecutableMetadata {
            path: path.to_path_buf(),
            source,
        })?;
    if !metadata.is_file() {
        return Err(RunnerResolveError::ExecutableUnavailable(
            path.to_path_buf(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Windows PowerShell Runner 的真实系统解析与稳定参数测试。

    use std::ffi::{OsStr, OsString};
    use std::sync::Mutex;

    use super::{
        CmdRunner, ProcessLaunchEnvironment, RunnerType, WindowsPowerShellRunner,
        CMD_ARTIFACT_ENVIRONMENT_NAME, CMD_CHCP_ENVIRONMENT_NAME, CMD_SYSTEM_ROOT_ENVIRONMENT_NAME,
    };
    use crate::execution::artifact::{MaterializedScript, RenderedScript};

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
        assert_eq!(runner.runner_type().as_str(), "windowsPowerShell");
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
        let artifact = MaterializedScript::create(RenderedScript::windows_powershell("exit 0"))
            .expect("应创建测试脚本");
        let script_path = artifact.script_path().to_path_buf();
        let working_directory = std::env::temp_dir();
        let executable = runner.executable().to_path_buf();
        let launch = runner.process_launch(artifact, &working_directory);

        assert_eq!(launch.executable(), executable);
        assert_eq!(launch.working_directory(), working_directory);
        assert_eq!(
            launch.arguments(),
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
        assert_eq!(launch.raw_command_tail(), None);
        assert_eq!(launch.environment(), &ProcessLaunchEnvironment::Inherit);
    }

    /// 验证 CMD 与 chcp 都从 System32 解析，且伪造 PATH 不影响固定路径和参数。
    #[test]
    fn resolves_cmd_and_chcp_without_path() {
        let _lock = PATH_ENVIRONMENT_LOCK
            .lock()
            .expect("PATH 测试互斥锁不应中毒");
        let baseline = CmdRunner::resolve().expect("系统应提供 CMD 与 chcp");
        let fake_path = OsStr::new(r"Z:\CmdBox\fake-cmd-and-chcp");
        let _environment = PathEnvironmentGuard::replace(fake_path);
        let resolved = CmdRunner::resolve().expect("CMD 解析不应依赖 PATH");

        assert_eq!(resolved.executable(), baseline.executable());
        assert_eq!(resolved.runner_type(), RunnerType::Cmd);
        assert_eq!(resolved.runner_type().as_str(), "cmd");
        assert_eq!(
            resolved
                .executable()
                .file_name()
                .and_then(|name| name.to_str()),
            Some("cmd.exe")
        );
        let chcp = resolved
            .fixed_environment()
            .get(CMD_CHCP_ENVIRONMENT_NAME)
            .expect("CMD 应绑定绝对 chcp");
        assert!(std::path::Path::new(chcp).is_absolute());
        assert!(std::path::Path::new(chcp).is_file());
        let system_root = resolved
            .fixed_environment()
            .get(CMD_SYSTEM_ROOT_ENVIRONMENT_NAME)
            .expect("CMD 应绑定确定 SystemRoot");
        assert!(std::path::Path::new(system_root).is_absolute());
        assert!(std::path::Path::new(system_root).is_dir());
    }

    /// 验证 CMD 固定 flags/raw tail 与 launch-only Artifact 环境绑定均不漂移。
    #[test]
    fn builds_stable_cmd_raw_tail_and_replacement_environment() {
        let runner = CmdRunner::resolve().expect("系统应提供 CMD 与 chcp");
        assert_eq!(
            runner.fixed_arguments(),
            ["/D", "/Q", "/A", "/E:ON", "/V:ON", "/S", "/C"].map(OsString::from)
        );
        assert_eq!(
            runner.raw_command_tail(),
            Some(&OsString::from("\"\"!CMDBOX_INTERNAL_ARTIFACT!\"\""))
        );

        let artifact = MaterializedScript::create(RenderedScript::cmd("@echo off\r\n"))
            .expect("应创建 CMD 测试 Artifact");
        let script_path = artifact.script_path().as_os_str().to_owned();
        let launch = runner.process_launch_with_environment(
            artifact,
            &std::env::temp_dir(),
            std::collections::BTreeMap::from([(
                CMD_ARTIFACT_ENVIRONMENT_NAME.to_owned(),
                OsString::from("attacker-controlled"),
            )]),
        );

        assert_eq!(launch.arguments().len(), 7);
        assert_eq!(
            launch.raw_command_tail(),
            Some(&OsString::from("\"\"!CMDBOX_INTERNAL_ARTIFACT!\"\""))
        );
        let ProcessLaunchEnvironment::Replace(environment) = launch.environment() else {
            panic!("CMD 必须使用完全替换环境");
        };
        assert_eq!(
            environment.get(CMD_ARTIFACT_ENVIRONMENT_NAME),
            Some(&script_path)
        );
        assert!(environment.contains_key(CMD_CHCP_ENVIRONMENT_NAME));
        assert!(environment.contains_key(CMD_SYSTEM_ROOT_ENVIRONMENT_NAME));
    }
}
