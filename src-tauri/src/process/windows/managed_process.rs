//! Windows Job Object 受管进程的创建、等待与整树终止。
//!
//! 本文件直接使用 Win32 `CreateProcessW`，先挂起创建 PowerShell，再加入设置了
//! `KILL_ON_JOB_CLOSE` 的独立 Job，最后恢复主线程，从根源上避免简单 spawn 后分配 Job 的
//! 子进程逃逸竞态。本原子尚不重定向 stdout/stderr；输出管道由下一原子接入。

use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::{Display, Formatter};
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::null;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_FAILED, WAIT_OBJECT_0};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, GetExitCodeProcess, ResumeThread, TerminateProcess, WaitForSingleObject,
    CREATE_NO_WINDOW, CREATE_SUSPENDED, INFINITE, PROCESS_INFORMATION, STARTUPINFOW,
};

use crate::execution::artifact::{ArtifactError, PowerShellArtifact};
use crate::process::windows::runner::WindowsPowerShellRunner;

/// CmdBox 主动取消 Job 时使用的进程退出码。
const CMDBOX_CANCEL_EXIT_CODE: u32 = 0xC000_013A;

/// `OpenProcess` 仅等待进程退出所需的标准访问权限。
#[cfg(test)]
const PROCESS_SYNCHRONIZE_ACCESS: u32 = 0x0010_0000;

/// Win32 受管进程操作的稳定阶段标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedProcessOperation {
    /// 创建 Job Object。
    CreateJob,
    /// 设置 Job 的 KILL_ON_JOB_CLOSE 限制。
    ConfigureJob,
    /// 以挂起状态创建 PowerShell 进程。
    CreateProcess,
    /// 把挂起进程分配到 Job。
    AssignProcess,
    /// 恢复已经受管的主线程。
    ResumeProcess,
    /// 等待受管根进程退出。
    WaitProcess,
    /// 读取根进程 Exit Code。
    ReadExitCode,
    /// 终止整个 Job 进程树。
    TerminateJob,
}

/// 输出受管进程操作的稳定开发者标识。
impl Display for ManagedProcessOperation {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::CreateJob => "createJob",
            Self::ConfigureJob => "configureJob",
            Self::CreateProcess => "createProcess",
            Self::AssignProcess => "assignProcess",
            Self::ResumeProcess => "resumeProcess",
            Self::WaitProcess => "waitProcess",
            Self::ReadExitCode => "readExitCode",
            Self::TerminateJob => "terminateJob",
        };
        formatter.write_str(value)
    }
}

/// Windows 受管进程失败。
#[derive(Debug)]
pub enum ManagedProcessError {
    /// 启动前 Artifact 完整性复验失败。
    Artifact(ArtifactError),
    /// Win32 进程或 Job 操作失败。
    Win32 {
        /// 失败发生的稳定操作。
        operation: ManagedProcessOperation,
        /// Windows 返回的原始系统错误。
        source: io::Error,
    },
    /// 工作目录不是可用的绝对目录。
    InvalidWorkingDirectory {
        /// 被拒绝的工作目录。
        path: PathBuf,
    },
}

/// 输出面向开发者的受管进程错误说明。
impl Display for ManagedProcessError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Artifact(source) => write!(formatter, "启动前 Artifact 复验失败：{source}"),
            Self::Win32 { operation, source } => {
                write!(formatter, "Windows 受管进程操作 {operation} 失败：{source}")
            }
            Self::InvalidWorkingDirectory { path } => {
                write!(formatter, "受管进程工作目录不可用：{}", path.display())
            }
        }
    }
}

/// 暴露 Artifact 或 Win32 的底层错误来源。
impl Error for ManagedProcessError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Artifact(source) => Some(source),
            Self::Win32 { source, .. } => Some(source),
            Self::InvalidWorkingDirectory { .. } => None,
        }
    }
}

/// 一个由 RAII 唯一拥有的 Win32 Handle。
#[derive(Debug)]
struct OwnedHandle {
    /// 非空且尚未关闭的原始 Handle。
    raw: HANDLE,
}

impl OwnedHandle {
    /// 接管一个已经由 Win32 API 成功创建的非空 Handle。
    fn new(raw: HANDLE) -> Self {
        debug_assert!(!raw.is_null());
        Self { raw }
    }

    /// 返回借用的原始 Handle，不转移关闭责任。
    fn raw(&self) -> HANDLE {
        self.raw
    }
}

/// 关闭当前对象唯一拥有的 Win32 Handle。
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: `OwnedHandle` 只由成功返回的非空句柄构造，且字段私有、不可复制，因此只关闭一次。
        unsafe {
            CloseHandle(self.raw);
        }
    }
}

/// 从 CreateProcessW 成功到 Resume 成功之间持有失败清理责任的挂起进程守卫。
#[derive(Debug)]
struct PendingStartupProcess {
    /// 尚未转交给 ManagedProcess 的根进程 Handle。
    process: Option<OwnedHandle>,
    /// 尚未关闭的挂起主线程 Handle。
    thread: Option<OwnedHandle>,
    /// CreateProcessW 返回的根进程 PID。
    process_id: u32,
}

impl PendingStartupProcess {
    /// 接管 CreateProcessW 返回的进程与主线程句柄，并默认承担失败终止责任。
    fn new(process_info: PROCESS_INFORMATION) -> Self {
        Self {
            process: Some(OwnedHandle::new(process_info.hProcess)),
            thread: Some(OwnedHandle::new(process_info.hThread)),
            process_id: process_info.dwProcessId,
        }
    }

    /// 返回挂起根进程 Handle，供 Job 分配使用。
    fn process_handle(&self) -> HANDLE {
        self.process.as_ref().expect("进程句柄尚未转交").raw()
    }

    /// 返回挂起主线程 Handle，供 ResumeThread 使用。
    fn thread_handle(&self) -> HANDLE {
        self.thread.as_ref().expect("线程句柄尚未关闭").raw()
    }

    /// 返回根进程 PID，供状态观测和失败注入测试记录。
    fn process_id(&self) -> u32 {
        self.process_id
    }

    /// 在 Assign 与 Resume 均成功后转交进程句柄，并关闭不再需要的主线程句柄。
    fn complete(mut self) -> (OwnedHandle, u32) {
        let process = self.process.take().expect("进程句柄只能转交一次");
        let thread = self.thread.take().expect("线程句柄只能关闭一次");
        drop(thread);
        (process, self.process_id)
    }
}

/// 启动未完成时先终止挂起进程并等待退出，再由字段 Drop 关闭两个句柄。
impl Drop for PendingStartupProcess {
    fn drop(&mut self) {
        let Some(process) = &self.process else {
            return;
        };
        // SAFETY: 守卫仍唯一持有有效进程 Handle；进程尚未完成受管启动，必须在返回错误前
        // 终止。即使 TerminateProcess 失败，后续若已成功 Assign，Job Drop 仍会再次清理。
        unsafe {
            TerminateProcess(process.raw(), CMDBOX_CANCEL_EXIT_CODE);
            WaitForSingleObject(process.raw(), 5_000);
        }
    }
}

/// 设置 KILL_ON_JOB_CLOSE 的独立 Execution Job。
#[derive(Debug)]
struct KillOnCloseJob {
    /// Job Object 的唯一所有权句柄。
    handle: OwnedHandle,
}

impl KillOnCloseJob {
    /// 创建 Job 并立即设置 KILL_ON_JOB_CLOSE，不启用任何 Breakaway 标志。
    fn create() -> Result<Self, ManagedProcessError> {
        // SAFETY: 使用空安全属性和匿名名称创建当前进程私有 Job。
        let raw = unsafe { CreateJobObjectW(null(), null()) };
        if raw.is_null() {
            return Err(last_win32_error(ManagedProcessOperation::CreateJob));
        }
        let handle = OwnedHandle::new(raw);

        // SAFETY: 结构体可零初始化，随后只设置文档要求的 LimitFlags；传入尺寸与类型一致。
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `handle` 和 `limits` 在调用期间有效，信息类与结构体类型匹配。
        let configured = unsafe {
            SetInformationJobObject(
                handle.raw(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            return Err(last_win32_error(ManagedProcessOperation::ConfigureJob));
        }

        Ok(Self { handle })
    }

    /// 把仍处于挂起状态的根进程加入当前 Job。
    fn assign(&self, process: HANDLE) -> Result<(), ManagedProcessError> {
        // SAFETY: 两个句柄均在调用期间有效；根进程尚未 Resume，不会先创建子进程。
        if unsafe { AssignProcessToJobObject(self.handle.raw(), process) } == 0 {
            return Err(last_win32_error(ManagedProcessOperation::AssignProcess));
        }
        Ok(())
    }

    /// 终止当前 Job 中的全部受管进程；返回只代表系统接受了终止操作。
    fn terminate(&self) -> Result<(), ManagedProcessError> {
        // SAFETY: Job 句柄在当前对象生命周期内有效。
        if unsafe { TerminateJobObject(self.handle.raw(), CMDBOX_CANCEL_EXIT_CODE) } == 0 {
            return Err(last_win32_error(ManagedProcessOperation::TerminateJob));
        }
        Ok(())
    }
}

/// 一个已经恢复运行且属于独立 Job Object 的根进程。
#[derive(Debug)]
pub struct ManagedProcess {
    /// 根进程 Handle，用于等待和读取 Exit Code。
    process: OwnedHandle,
    /// 持有整个受管进程树的 Job；Drop 时触发 KILL_ON_JOB_CLOSE。
    job: KillOnCloseJob,
    /// 根进程的系统 PID，只用于观测和测试，不提供按 PID 终止入口。
    process_id: u32,
    /// 保持临时脚本到进程对象结束，防止运行期间被提前清理。
    _artifact: PowerShellArtifact,
}

impl ManagedProcess {
    /// 复验 Artifact，并按“挂起创建 → 分配 Job → 恢复线程”的固定顺序启动进程。
    pub fn spawn(
        runner: &WindowsPowerShellRunner,
        artifact: PowerShellArtifact,
        working_directory: &Path,
    ) -> Result<Self, ManagedProcessError> {
        Self::spawn_with_job_assignment(
            runner,
            artifact,
            working_directory,
            |job, process, _process_id| job.assign(process),
        )
    }

    /// 使用给定 Job 分配动作完成受管启动；测试可注入 Assign 失败以证明守卫会清理 PID。
    fn spawn_with_job_assignment<F>(
        runner: &WindowsPowerShellRunner,
        artifact: PowerShellArtifact,
        working_directory: &Path,
        assign: F,
    ) -> Result<Self, ManagedProcessError>
    where
        F: FnOnce(&KillOnCloseJob, HANDLE, u32) -> Result<(), ManagedProcessError>,
    {
        if !working_directory.is_absolute() || !working_directory.is_dir() {
            return Err(ManagedProcessError::InvalidWorkingDirectory {
                path: working_directory.to_path_buf(),
            });
        }

        // 完整性复验必须紧邻创建 Job/Process，失败时不会取得任何可运行进程资源。
        artifact
            .verify_before_spawn()
            .map_err(ManagedProcessError::Artifact)?;
        let job = KillOnCloseJob::create()?;
        let mut command_line = build_command_line(
            runner.executable(),
            &runner.script_arguments(artifact.script_path()),
        );
        let application_name = wide_null_terminated(runner.executable().as_os_str());
        let current_directory = wide_null_terminated(working_directory.as_os_str());
        let mut startup_info: STARTUPINFOW = unsafe { zeroed() };
        startup_info.cb = size_of::<STARTUPINFOW>() as u32;
        let mut process_info: PROCESS_INFORMATION = unsafe { zeroed() };

        // SAFETY: 路径缓冲区均以 NUL 结尾并在调用期间有效；命令行是可写 UTF-16；不继承
        // Handle；STARTUPINFO/PROCESS_INFORMATION 尺寸正确。进程以挂起状态返回。
        let created = unsafe {
            CreateProcessW(
                application_name.as_ptr(),
                command_line.as_mut_ptr(),
                null(),
                null(),
                0,
                CREATE_SUSPENDED | CREATE_NO_WINDOW,
                null(),
                current_directory.as_ptr(),
                &startup_info,
                &mut process_info,
            )
        };
        if created == 0 {
            return Err(last_win32_error(ManagedProcessOperation::CreateProcess));
        }

        let pending = PendingStartupProcess::new(process_info);
        assign(&job, pending.process_handle(), pending.process_id())?;

        // SAFETY: 主线程 Handle 有效且进程仍保持 CreateProcessW 建立的首次挂起计数。
        let previous_suspend_count = unsafe { ResumeThread(pending.thread_handle()) };
        if previous_suspend_count == u32::MAX {
            return Err(last_win32_error(ManagedProcessOperation::ResumeProcess));
        }
        let (process, process_id) = pending.complete();

        Ok(Self {
            process,
            job,
            process_id,
            _artifact: artifact,
        })
    }

    /// 返回受管根进程 PID，仅供状态观测；取消仍必须按 Execution Job 进行。
    pub fn process_id(&self) -> u32 {
        self.process_id
    }

    /// 阻塞等待根进程结束并返回原始 Exit Code；后续会话层会把它放入专用等待任务。
    pub fn wait(&self) -> Result<u32, ManagedProcessError> {
        // SAFETY: 根进程 Handle 在等待期间有效，INFINITE 表示事件驱动等待而不是轮询。
        let wait_result = unsafe { WaitForSingleObject(self.process.raw(), INFINITE) };
        if wait_result == WAIT_FAILED {
            return Err(last_win32_error(ManagedProcessOperation::WaitProcess));
        }
        if wait_result != WAIT_OBJECT_0 {
            return Err(ManagedProcessError::Win32 {
                operation: ManagedProcessOperation::WaitProcess,
                source: io::Error::other(format!("unexpected wait result: {wait_result}")),
            });
        }

        let mut exit_code = 0_u32;
        // SAFETY: 等待已确认根进程退出，输出指针指向有效 u32。
        if unsafe { GetExitCodeProcess(self.process.raw(), &mut exit_code) } == 0 {
            return Err(last_win32_error(ManagedProcessOperation::ReadExitCode));
        }
        Ok(exit_code)
    }

    /// 请求 Windows 终止整个 Execution Job；调用方必须继续 wait 后才能发布 Cancelled。
    pub fn terminate_job(&self) -> Result<(), ManagedProcessError> {
        self.job.terminate()
    }
}

/// 读取最近 Win32 错误并绑定到稳定操作。
fn last_win32_error(operation: ManagedProcessOperation) -> ManagedProcessError {
    ManagedProcessError::Win32 {
        operation,
        source: io::Error::last_os_error(),
    }
}

/// 构造带 NUL 结尾的 Windows UTF-16 字符串。
fn wide_null_terminated(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

/// 按 Windows C Runtime 规则引用一个命令行参数，保持空格、引号和尾部反斜杠语义。
fn quote_windows_argument(argument: &OsStr) -> Vec<u16> {
    let input: Vec<u16> = argument.encode_wide().collect();
    let needs_quotes = input.is_empty()
        || input
            .iter()
            .any(|character| matches!(*character, 0x20 | 0x09 | 0x22));
    if !needs_quotes {
        return input;
    }

    let mut output = vec![b'"' as u16];
    let mut backslashes = 0_usize;
    for character in input {
        if character == b'\\' as u16 {
            backslashes += 1;
            continue;
        }
        if character == b'"' as u16 {
            output.extend(std::iter::repeat_n(b'\\' as u16, backslashes * 2 + 1));
            output.push(character);
            backslashes = 0;
            continue;
        }
        output.extend(std::iter::repeat_n(b'\\' as u16, backslashes));
        backslashes = 0;
        output.push(character);
    }
    output.extend(std::iter::repeat_n(b'\\' as u16, backslashes * 2));
    output.push(b'"' as u16);
    output
}

/// 为 CreateProcessW 构造包含 argv[0]、固定参数和脚本路径的可写命令行。
fn build_command_line(executable: &Path, arguments: &[OsString]) -> Vec<u16> {
    let mut command_line = quote_windows_argument(executable.as_os_str());
    for argument in arguments {
        command_line.push(b' ' as u16);
        command_line.extend(quote_windows_argument(argument));
    }
    command_line.push(0);
    command_line
}

#[cfg(test)]
mod tests {
    //! Job Object 受管进程的自然退出和整树取消测试。

    use std::fs;
    use std::path::Path;
    use std::thread;
    use std::time::{Duration, Instant};
    use std::{
        io,
        sync::atomic::{AtomicU32, Ordering},
    };

    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject};

    use super::{
        ManagedProcess, ManagedProcessError, ManagedProcessOperation, PROCESS_SYNCHRONIZE_ACCESS,
    };
    use crate::execution::artifact::PowerShellArtifact;
    use crate::process::windows::runner::WindowsPowerShellRunner;

    /// PowerShell 进程使用的安全临时工作目录。
    fn safe_working_directory() -> std::path::PathBuf {
        std::env::temp_dir()
    }

    /// 在限定时间内等待测试脚本写出子进程 PID。
    fn wait_for_pid_file(path: &Path) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(value) = fs::read_to_string(path) {
                if let Ok(pid) = value.trim().parse() {
                    return pid;
                }
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!("测试脚本未在期限内写出子进程 PID：{}", path.display());
    }

    /// 等待指定 PID 对应的进程退出；进程已经不存在也视为成功。
    fn wait_until_process_exits(pid: u32) {
        // SAFETY: 只请求同步等待权限；返回空句柄表示进程已经退出或不可访问，本测试目标已满足。
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE_ACCESS, 0, pid) };
        if handle.is_null() {
            return;
        }
        // SAFETY: OpenProcess 返回的句柄有效，等待后由当前函数关闭一次。
        let wait_result = unsafe { WaitForSingleObject(handle, 10_000) };
        unsafe {
            CloseHandle(handle);
        }
        assert_ne!(wait_result, WAIT_TIMEOUT, "PID {pid} 未按期退出");
        assert_eq!(wait_result, WAIT_OBJECT_0, "PID {pid} 等待失败");
    }

    /// 验证 Exit Code 0 的固定脚本可自然结束。
    #[test]
    fn managed_process_waits_for_natural_exit() {
        let runner = WindowsPowerShellRunner::resolve().expect("系统应提供 Windows PowerShell");
        let artifact = PowerShellArtifact::create("exit 0").expect("应创建测试脚本");
        let working_directory = safe_working_directory();
        let process =
            ManagedProcess::spawn(&runner, artifact, &working_directory).expect("应启动受管进程");

        assert_eq!(process.wait().expect("应等待自然退出"), 0);
    }

    /// 验证 TerminateJobObject 会同时终止 PowerShell 根进程和它创建的子进程。
    #[test]
    fn terminate_job_stops_root_and_child_process() {
        let runner = WindowsPowerShellRunner::resolve().expect("系统应提供 Windows PowerShell");
        let pid_file =
            std::env::temp_dir().join(format!("cmdbox-child-{}.pid", uuid::Uuid::new_v4()));
        let escaped_pid_file = pid_file.to_string_lossy().replace('\'', "''");
        let script = format!(
            "$child = Start-Process -FilePath $env:ComSpec -ArgumentList '/d /c ping -t 127.0.0.1' -PassThru; Set-Content -LiteralPath '{escaped_pid_file}' -Value $child.Id; Wait-Process -Id $child.Id"
        );
        let artifact = PowerShellArtifact::create(&script).expect("应创建测试脚本");
        let working_directory = safe_working_directory();
        let process =
            ManagedProcess::spawn(&runner, artifact, &working_directory).expect("应启动受管进程");
        let root_pid = process.process_id();
        let child_pid = wait_for_pid_file(&pid_file);

        process.terminate_job().expect("应接受整树终止请求");
        let _exit_code = process.wait().expect("终止后应确认根进程结束");
        wait_until_process_exits(root_pid);
        wait_until_process_exits(child_pid);
        let _ = fs::remove_file(pid_file);
    }

    /// 验证 Job 分配失败时启动守卫会终止尚未受管的挂起 PowerShell，不留下孤立 PID。
    #[test]
    fn assign_failure_terminates_pending_suspended_process() {
        let runner = WindowsPowerShellRunner::resolve().expect("系统应提供 Windows PowerShell");
        let artifact =
            PowerShellArtifact::create("Start-Sleep -Seconds 60").expect("应创建测试脚本");
        let working_directory = safe_working_directory();
        let captured_pid = AtomicU32::new(0);

        let result = ManagedProcess::spawn_with_job_assignment(
            &runner,
            artifact,
            &working_directory,
            |_job, _process, process_id| {
                captured_pid.store(process_id, Ordering::SeqCst);
                Err(ManagedProcessError::Win32 {
                    operation: ManagedProcessOperation::AssignProcess,
                    source: io::Error::other("测试注入 Job 分配失败"),
                })
            },
        );

        assert!(matches!(
            result,
            Err(ManagedProcessError::Win32 {
                operation: ManagedProcessOperation::AssignProcess,
                ..
            })
        ));
        let pid = captured_pid.load(Ordering::SeqCst);
        assert_ne!(pid, 0, "失败注入前应取得 CreateProcessW 返回的 PID");
        wait_until_process_exits(pid);
    }
}
