//! Windows Job Object 受管进程的创建、等待与整树终止。
//!
//! 本文件只消费字段私有的 `ProcessLaunch`，直接使用 Win32 `CreateProcessW` 先挂起创建
//! 已解析进程，再加入设置了
//! `KILL_ON_JOB_CLOSE` 的独立 Job，最后恢复主线程，从根源上避免简单 spawn 后分配 Job 的
//! 子进程逃逸竞态。Runner、脚本类型和临时 Artifact 均封装在启动值内；stdout/stderr 管道
//! 和取消句柄可在 Resume 前交给 Session 完成预绑定。

use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::{Display, Formatter};
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::Arc;

use windows_sys::Win32::Foundation::{
    CloseHandle, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
    WAIT_FAILED, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectAssociateCompletionPortInformation,
    JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
    JOBOBJECT_ASSOCIATE_COMPLETION_PORT, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::SystemServices::JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, GetExitCodeProcess, ResumeThread, TerminateProcess, WaitForSingleObject,
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, INFINITE, PROCESS_INFORMATION,
    STARTF_USESTDHANDLES, STARTUPINFOW,
};
use windows_sys::Win32::System::IO::{CreateIoCompletionPort, GetQueuedCompletionStatus};

use crate::execution::artifact::ArtifactError;
use crate::process::windows::runner::{ProcessLaunch, ProcessLaunchEnvironment};

/// CreateProcessW 环境块允许的最大 UTF-16 单元数，包含最终双 NUL。
const WINDOWS_ENVIRONMENT_MAX_UTF16_UNITS: usize = 32_767;

/// CmdBox 主动取消 Job 时使用的进程退出码。
pub(crate) const CMDBOX_CANCEL_EXIT_CODE: u32 = 0xC000_013A;

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
    /// 创建并关联 Job 完成端口。
    ConfigureCompletionPort,
    /// 以挂起状态创建已解析的受管进程。
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
    /// 等待 Job 报告 Active Process Zero。
    WaitJobEmpty,
}

/// 输出受管进程操作的稳定开发者标识。
impl Display for ManagedProcessOperation {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::CreateJob => "createJob",
            Self::ConfigureJob => "configureJob",
            Self::ConfigureCompletionPort => "configureCompletionPort",
            Self::CreateProcess => "createProcess",
            Self::AssignProcess => "assignProcess",
            Self::ResumeProcess => "resumeProcess",
            Self::WaitProcess => "waitProcess",
            Self::ReadExitCode => "readExitCode",
            Self::TerminateJob => "terminateJob",
            Self::WaitJobEmpty => "waitJobEmpty",
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
    /// 内部环境覆盖无法编码为合法的 Windows Unicode 环境块。
    InvalidEnvironment,
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
            Self::InvalidEnvironment => formatter.write_str("受管进程环境块无效"),
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
            Self::InvalidEnvironment => None,
        }
    }
}

/// 一个由 RAII 唯一拥有的 Win32 Handle。
#[derive(Debug)]
struct OwnedHandle {
    /// 非空且尚未关闭的原始 Handle。
    raw: HANDLE,
}

// SAFETY: Windows 内核 Handle 可以跨线程传递并由多个线程并发用于等待或 Job 操作；
// `OwnedHandle` 仍保持唯一关闭所有权，且只公开不转移所有权的原子 Win32 调用。
unsafe impl Send for OwnedHandle {}
// SAFETY: 同上；并发借用不会改变句柄值或产生重复 CloseHandle。
unsafe impl Sync for OwnedHandle {}

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

    /// 把 Pipe 读端句柄转交给标准库 File，由 File 负责后续关闭。
    fn into_file(self) -> std::fs::File {
        let raw = self.raw;
        std::mem::forget(self);
        // SAFETY: 句柄由 CreatePipe 创建且所有权刚从 OwnedHandle 转移，File 将唯一关闭它。
        unsafe { std::fs::File::from_raw_handle(raw as RawHandle) }
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

/// 父进程持有的 stdout/stderr Pipe 读端。
#[derive(Debug)]
pub struct CapturedOutput {
    /// 受管进程 stdout 的唯一读端。
    stdout: std::fs::File,
    /// 受管进程 stderr 的唯一读端。
    stderr: std::fs::File,
}

impl CapturedOutput {
    /// 把两个读端交给独立 Reader 线程。
    pub fn into_readers(self) -> (std::fs::File, std::fs::File) {
        (self.stdout, self.stderr)
    }
}

/// CreateProcessW 前创建的三组标准流 Pipe。
#[derive(Debug)]
struct StandardPipes {
    /// 父进程读取 stdout 的非继承端。
    stdout_read: OwnedHandle,
    /// 子进程写入 stdout 的继承端。
    stdout_write: OwnedHandle,
    /// 父进程读取 stderr 的非继承端。
    stderr_read: OwnedHandle,
    /// 子进程写入 stderr 的继承端。
    stderr_write: OwnedHandle,
    /// 子进程读取 stdin 的继承端；父端关闭后始终得到 EOF。
    stdin_read: OwnedHandle,
    /// 父进程持有但不写入的 stdin 端。
    stdin_write: OwnedHandle,
}

impl StandardPipes {
    /// 创建标准流 Pipe，并确保只有子进程所需的三个端可继承。
    fn create() -> Result<Self, ManagedProcessError> {
        let (stdout_read, stdout_write) = create_inherited_pipe()?;
        let (stderr_read, stderr_write) = create_inherited_pipe()?;
        let (stdin_read, stdin_write) = create_inherited_stdin_pipe()?;
        Ok(Self {
            stdout_read,
            stdout_write,
            stderr_read,
            stderr_write,
            stdin_read,
            stdin_write,
        })
    }

    /// 在进程成功创建后关闭父进程不需要的子端，并转交两个输出读端。
    fn into_captured_output(self) -> CapturedOutput {
        drop(self.stdout_write);
        drop(self.stderr_write);
        drop(self.stdin_read);
        drop(self.stdin_write);
        CapturedOutput {
            stdout: self.stdout_read.into_file(),
            stderr: self.stderr_read.into_file(),
        }
    }
}

/// 设置 KILL_ON_JOB_CLOSE 的独立 Execution Job。
#[derive(Debug)]
struct KillOnCloseJob {
    /// Job Object 的唯一所有权句柄。
    handle: OwnedHandle,
    /// 接收当前 Job ACTIVE_PROCESS_ZERO 通知的独立完成端口。
    completion_port: OwnedHandle,
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

        // SAFETY: INVALID_HANDLE_VALUE 表示只创建新的完成端口；并发线程数 1 足够当前 Job
        // 的单一 Supervisor 消费通知。
        let completion_port =
            unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, null_mut(), 0, 1) };
        if completion_port.is_null() {
            return Err(last_win32_error(
                ManagedProcessOperation::ConfigureCompletionPort,
            ));
        }

        Ok(Self {
            handle,
            completion_port: OwnedHandle::new(completion_port),
        })
    }

    /// 把仍处于挂起状态的根进程加入当前 Job。
    fn assign(&self, process: HANDLE) -> Result<(), ManagedProcessError> {
        // SAFETY: 两个句柄均在调用期间有效；根进程尚未 Resume，不会先创建子进程。
        if unsafe { AssignProcessToJobObject(self.handle.raw(), process) } == 0 {
            return Err(last_win32_error(ManagedProcessOperation::AssignProcess));
        }
        let association = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
            CompletionKey: self.handle.raw(),
            CompletionPort: self.completion_port.raw(),
        };
        // SAFETY: 根进程已经加入 Job 但仍处于挂起状态，因此从关联完成端口到 Resume 之间
        // 不会产生子进程竞态；结构体、Job 和完成端口句柄在调用期间均有效。
        if unsafe {
            SetInformationJobObject(
                self.handle.raw(),
                JobObjectAssociateCompletionPortInformation,
                (&association as *const JOBOBJECT_ASSOCIATE_COMPLETION_PORT).cast(),
                size_of::<JOBOBJECT_ASSOCIATE_COMPLETION_PORT>() as u32,
            )
        } == 0
        {
            return Err(last_win32_error(
                ManagedProcessOperation::ConfigureCompletionPort,
            ));
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

    /// 事件驱动等待 Job 的 Active Process 数降为零，明确确认整个受管树已经结束。
    fn wait_until_empty(&self) -> Result<(), ManagedProcessError> {
        loop {
            let mut message = 0_u32;
            let mut completion_key = 0_usize;
            let mut overlapped = null_mut();
            // SAFETY: 完成端口句柄和三个输出指针在阻塞调用期间有效；INFINITE 表示由 Job
            // 通知唤醒而非轮询。每个 Job 只有当前 Supervisor 消费该端口。
            let dequeued = unsafe {
                GetQueuedCompletionStatus(
                    self.completion_port.raw(),
                    &mut message,
                    &mut completion_key,
                    &mut overlapped,
                    INFINITE,
                )
            };
            if dequeued == 0 {
                return Err(last_win32_error(ManagedProcessOperation::WaitJobEmpty));
            }
            if message == JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO {
                return Ok(());
            }
        }
    }
}

/// 可在会话管理线程中独立请求整树终止的 Job 句柄所有权。
#[derive(Debug, Clone)]
pub struct ManagedProcessCancellation {
    /// 与受管进程共享同一个 KILL_ON_JOB_CLOSE Job。
    job: Arc<KillOnCloseJob>,
}

impl ManagedProcessCancellation {
    /// 请求终止当前 Execution 的整个 Job；返回只代表系统接受终止操作。
    pub fn terminate_job(&self) -> Result<(), ManagedProcessError> {
        self.job.terminate()
    }
}

/// 已经挂起创建并加入 Job、但尚未恢复主线程的受管进程。
#[derive(Debug)]
pub struct PreparedManagedProcess {
    /// Resume 前失败时负责终止并回收挂起根进程。
    pending: PendingStartupProcess,
    /// 与取消入口共享的 KILL_ON_JOB_CLOSE Job。
    job: Arc<KillOnCloseJob>,
    /// 保持完整启动参数与临时脚本租约直到受管进程生命周期结束。
    launch: ProcessLaunch,
    /// Resume 前即可交给输出 Reader 的 stdout/stderr 读端。
    output: Option<CapturedOutput>,
}

impl PreparedManagedProcess {
    /// 返回挂起根进程 PID，仅供会话 Started 事件观测。
    pub fn process_id(&self) -> u32 {
        self.pending.process_id()
    }

    /// 返回独立取消入口，允许 Manager 在 Resume 前登记 Active Execution。
    pub fn cancellation(&self) -> ManagedProcessCancellation {
        ManagedProcessCancellation {
            job: Arc::clone(&self.job),
        }
    }

    /// 取出 stdout/stderr 读端并预先绑定 Reader；只能转交一次。
    pub fn take_output(&mut self) -> Option<CapturedOutput> {
        self.output.take()
    }

    /// 恢复已经加入 Job 的主线程，并转成可等待的运行中进程。
    pub fn resume(self) -> Result<ManagedProcess, ManagedProcessError> {
        // SAFETY: 主线程 Handle 有效且进程仍保持 CreateProcessW 建立的首次挂起计数。
        let previous_suspend_count = unsafe { ResumeThread(self.pending.thread_handle()) };
        if previous_suspend_count == u32::MAX {
            return Err(last_win32_error(ManagedProcessOperation::ResumeProcess));
        }
        let (process, process_id) = self.pending.complete();

        Ok(ManagedProcess {
            process,
            job: self.job,
            process_id,
            _launch: self.launch,
        })
    }
}

/// 一个已经恢复运行且属于独立 Job Object 的根进程。
#[derive(Debug)]
pub struct ManagedProcess {
    /// 根进程 Handle，用于等待和读取 Exit Code。
    process: OwnedHandle,
    /// 持有整个受管进程树的 Job；Drop 时触发 KILL_ON_JOB_CLOSE。
    job: Arc<KillOnCloseJob>,
    /// 根进程的系统 PID，只用于观测和测试，不提供按 PID 终止入口。
    process_id: u32,
    /// 保持完整启动值与临时脚本租约到进程对象结束，防止运行期间被提前清理。
    _launch: ProcessLaunch,
}

impl ManagedProcess {
    /// 复验 Artifact，并按“挂起创建 → 分配 Job → 恢复线程”的固定顺序启动进程。
    pub fn spawn(launch: ProcessLaunch) -> Result<Self, ManagedProcessError> {
        Self::prepare_with_job_assignment(launch, |job, process, _process_id| job.assign(process))?
            .resume()
    }

    /// 复验 Artifact，挂起创建并加入 Job，但把 Resume 留给会话层完成预绑定后调用。
    pub fn prepare(launch: ProcessLaunch) -> Result<PreparedManagedProcess, ManagedProcessError> {
        Self::prepare_with_job_assignment(launch, |job, process, _process_id| job.assign(process))
    }

    /// 使用给定 Job 分配动作完成受管启动；测试可注入 Assign 失败以证明守卫会清理 PID。
    fn prepare_with_job_assignment<F>(
        launch: ProcessLaunch,
        assign: F,
    ) -> Result<PreparedManagedProcess, ManagedProcessError>
    where
        F: FnOnce(&KillOnCloseJob, HANDLE, u32) -> Result<(), ManagedProcessError>,
    {
        if !launch.working_directory().is_absolute() || !launch.working_directory().is_dir() {
            return Err(ManagedProcessError::InvalidWorkingDirectory {
                path: launch.working_directory().to_path_buf(),
            });
        }

        // 完整性复验必须紧邻创建 Job/Process，失败时不会取得任何可运行进程资源。
        launch
            .verify_before_spawn()
            .map_err(ManagedProcessError::Artifact)?;
        let job = Arc::new(KillOnCloseJob::create()?);
        let mut command_line = build_command_line(
            launch.executable(),
            launch.arguments(),
            launch.raw_command_tail(),
        );
        let application_name = wide_null_terminated(launch.executable().as_os_str());
        let current_directory = wide_null_terminated(launch.working_directory().as_os_str());
        let mut environment = match launch.environment() {
            ProcessLaunchEnvironment::Inherit => None,
            ProcessLaunchEnvironment::Replace(entries) => {
                Some(build_unicode_environment_block(entries)?)
            }
        };
        let creation_flags = CREATE_SUSPENDED
            | CREATE_NO_WINDOW
            | if environment.is_some() {
                CREATE_UNICODE_ENVIRONMENT
            } else {
                0
            };
        let environment_pointer = environment
            .as_mut()
            .map_or(null(), |block| block.as_mut_ptr().cast());
        let mut startup_info: STARTUPINFOW = unsafe { zeroed() };
        startup_info.cb = size_of::<STARTUPINFOW>() as u32;
        let pipes = StandardPipes::create()?;
        startup_info.dwFlags = STARTF_USESTDHANDLES;
        startup_info.hStdOutput = pipes.stdout_write.raw();
        startup_info.hStdError = pipes.stderr_write.raw();
        startup_info.hStdInput = pipes.stdin_read.raw();
        let mut process_info: PROCESS_INFORMATION = unsafe { zeroed() };

        // SAFETY: 路径缓冲区均以 NUL 结尾并在调用期间有效；命令行是可写 UTF-16；只继承
        // 标准流中显式保留继承位的 Handle；结构体尺寸正确。进程以挂起状态返回。
        let created = unsafe {
            CreateProcessW(
                application_name.as_ptr(),
                command_line.as_mut_ptr(),
                null(),
                null(),
                1,
                creation_flags,
                environment_pointer,
                current_directory.as_ptr(),
                &startup_info,
                &mut process_info,
            )
        };
        if created == 0 {
            return Err(last_win32_error(ManagedProcessOperation::CreateProcess));
        }

        let pending = PendingStartupProcess::new(process_info);
        let output = pipes.into_captured_output();
        assign(job.as_ref(), pending.process_handle(), pending.process_id())?;

        Ok(PreparedManagedProcess {
            pending,
            job,
            launch,
            output: Some(output),
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

    /// 等待 Job 完成端口明确报告 Active Process Zero。
    pub fn wait_job_empty(&self) -> Result<(), ManagedProcessError> {
        self.job.wait_until_empty()
    }
}

/// 创建匿名 Pipe，写端保留继承标志，读端清除继承标志供父进程持有。
fn create_inherited_pipe() -> Result<(OwnedHandle, OwnedHandle), ManagedProcessError> {
    let security = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    let mut read = null_mut();
    let mut write = null_mut();
    // SAFETY: 输出指针和安全属性在调用期间有效，系统返回两个新的 Pipe Handle。
    if unsafe { CreatePipe(&mut read, &mut write, &security, 0) } == 0 {
        return Err(last_win32_error(ManagedProcessOperation::CreateProcess));
    }
    let read = OwnedHandle::new(read);
    let write = OwnedHandle::new(write);
    // SAFETY: 读端 Handle 有效；清除继承位，避免子进程持有导致父 Reader 永远收不到 EOF。
    if unsafe { SetHandleInformation(read.raw(), HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(last_win32_error(ManagedProcessOperation::CreateProcess));
    }
    Ok((read, write))
}

/// 创建 stdin Pipe，保留子进程读端继承位并清除父进程写端继承位。
fn create_inherited_stdin_pipe() -> Result<(OwnedHandle, OwnedHandle), ManagedProcessError> {
    let security = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    let mut read = null_mut();
    let mut write = null_mut();
    // SAFETY: 输出指针和安全属性在调用期间有效，系统返回两个新的 Pipe Handle。
    if unsafe { CreatePipe(&mut read, &mut write, &security, 0) } == 0 {
        return Err(last_win32_error(ManagedProcessOperation::CreateProcess));
    }
    let read = OwnedHandle::new(read);
    let write = OwnedHandle::new(write);
    // SAFETY: 父进程写端 Handle 有效；清除继承位后子进程只继承 stdin 读端。父进程在
    // CreateProcessW 返回后关闭写端，子进程随即得到 EOF，符合 NonInteractive 契约。
    if unsafe { SetHandleInformation(write.raw(), HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(last_win32_error(ManagedProcessOperation::CreateProcess));
    }
    Ok((read, write))
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
fn build_command_line(
    executable: &Path,
    arguments: &[OsString],
    raw_command_tail: Option<&OsString>,
) -> Vec<u16> {
    let mut command_line = quote_windows_argument(executable.as_os_str());
    for argument in arguments {
        command_line.push(b' ' as u16);
        if raw_command_tail.is_some() {
            // CMD 使用自有命令行解析器；这里只消费 Runner 内固定 ASCII flags，不能套 CRT
            // 引号，否则 `cmd.exe` 会把带引号的 switch 当成待执行命令。
            command_line.extend(argument.encode_wide());
        } else {
            command_line.extend(quote_windows_argument(argument));
        }
    }
    if let Some(raw_command_tail) = raw_command_tail {
        command_line.push(b' ' as u16);
        command_line.extend(raw_command_tail.encode_wide());
    }
    command_line.push(0);
    command_line
}

/// 把完整替换环境编码为大小写不敏感排序、无冲突且双 NUL 结尾的 UTF-16 块。
fn build_unicode_environment_block(
    entries: &std::collections::BTreeMap<String, OsString>,
) -> Result<Vec<u16>, ManagedProcessError> {
    let mut sorted_entries = entries.iter().collect::<Vec<_>>();
    for (key, value) in &sorted_entries {
        if key.is_empty()
            || !key.is_ascii()
            || key.contains('=')
            || key.contains('\0')
            || value.encode_wide().any(|unit| unit == 0)
        {
            return Err(ManagedProcessError::InvalidEnvironment);
        }
    }
    sorted_entries.sort_by(|(left, _), (right, _)| {
        windows_environment_sort_key(left).cmp(&windows_environment_sort_key(right))
    });
    if sorted_entries
        .windows(2)
        .any(|pair| windows_environment_key_eq(pair[0].0, pair[1].0))
    {
        return Err(ManagedProcessError::InvalidEnvironment);
    }

    let mut block = Vec::new();
    for (key, value) in &sorted_entries {
        let key_units = key.encode_utf16().collect::<Vec<_>>();
        let value_units = value.encode_wide().collect::<Vec<_>>();
        block.extend(key_units);
        block.push(b'=' as u16);
        block.extend(value_units);
        block.push(0);
    }
    if sorted_entries.is_empty() {
        block.push(0);
    }
    block.push(0);
    if block.len() > WINDOWS_ENVIRONMENT_MAX_UTF16_UNITS {
        return Err(ManagedProcessError::InvalidEnvironment);
    }
    Ok(block)
}

/// 使用 Windows 环境名所需的 ASCII 大小写不敏感语义比较两个键。
fn windows_environment_key_eq(left: &str, right: &str) -> bool {
    windows_environment_sort_key(left) == windows_environment_sort_key(right)
}

/// 为环境块排序生成 Windows ASCII 大小写不敏感的稳定比较键。
fn windows_environment_sort_key(value: &str) -> Vec<u8> {
    value
        .bytes()
        .map(|byte| byte.to_ascii_uppercase())
        .collect()
}

#[cfg(test)]
mod tests {
    //! Job Object 受管进程的自然退出和整树取消测试。

    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::fs;
    use std::io::Read;
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
        build_unicode_environment_block, ManagedProcess, ManagedProcessError,
        ManagedProcessOperation, PROCESS_SYNCHRONIZE_ACCESS,
    };
    use crate::execution::artifact::{ArtifactError, MaterializedScript, RenderedScript};
    use crate::process::windows::runner::{
        CmdRunner, ProcessLaunch, WindowsPowerShellRunner, CMD_CHCP_ENVIRONMENT_NAME,
    };

    /// PowerShell 进程使用的安全临时工作目录。
    fn safe_working_directory() -> std::path::PathBuf {
        std::env::temp_dir()
    }

    /// 为测试脚本文本构造与正式 Session 相同的字段私有 PowerShell 启动值。
    fn powershell_launch(script: &str, working_directory: &Path) -> ProcessLaunch {
        let runner = WindowsPowerShellRunner::resolve().expect("系统应提供 Windows PowerShell");
        let rendered = RenderedScript::windows_powershell(script);
        let artifact = MaterializedScript::create(rendered).expect("应创建测试脚本");
        runner.process_launch(artifact, working_directory)
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
        let working_directory = safe_working_directory();
        let launch = powershell_launch("exit 0", &working_directory);
        let process = ManagedProcess::spawn(launch).expect("应启动受管进程");

        assert_eq!(process.wait().expect("应等待自然退出"), 0);
    }

    /// 验证启动值持有的脚本被篡改时不会创建进程，并由失败路径精确清理其唯一目录。
    #[test]
    fn rejects_tampered_process_launch_and_cleans_owned_artifact() {
        let runner = WindowsPowerShellRunner::resolve().expect("系统应提供 Windows PowerShell");
        let rendered = RenderedScript::windows_powershell("exit 0");
        let artifact = MaterializedScript::create(rendered).expect("应创建测试脚本");
        let execution_directory = artifact
            .script_path()
            .parent()
            .expect("测试脚本应位于唯一目录")
            .to_path_buf();
        fs::write(artifact.script_path(), b"exit 9").expect("测试应能篡改临时脚本");
        let working_directory = safe_working_directory();
        let launch = runner.process_launch(artifact, &working_directory);

        let result = ManagedProcess::spawn(launch);

        assert!(matches!(
            result,
            Err(ManagedProcessError::Artifact(
                ArtifactError::IntegrityMismatch { .. }
            ))
        ));
        assert!(!execution_directory.exists(), "失败时应清理当前唯一目录");
    }

    /// 验证 TerminateJobObject 会同时终止 PowerShell 根进程和它创建的子进程。
    #[test]
    fn terminate_job_stops_root_and_child_process() {
        let pid_file =
            std::env::temp_dir().join(format!("cmdbox-child-{}.pid", uuid::Uuid::new_v4()));
        let escaped_pid_file = pid_file.to_string_lossy().replace('\'', "''");
        let script = format!(
            "$child = Start-Process -FilePath $env:ComSpec -ArgumentList '/d /c ping -t 127.0.0.1' -PassThru; Set-Content -LiteralPath '{escaped_pid_file}' -Value $child.Id; Wait-Process -Id $child.Id"
        );
        let working_directory = safe_working_directory();
        let launch = powershell_launch(&script, &working_directory);
        let process = ManagedProcess::spawn(launch).expect("应启动受管进程");
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
        let working_directory = safe_working_directory();
        let launch = powershell_launch("Start-Sleep -Seconds 60", &working_directory);
        let captured_pid = AtomicU32::new(0);

        let result =
            ManagedProcess::prepare_with_job_assignment(launch, |_job, _process, process_id| {
                captured_pid.store(process_id, Ordering::SeqCst);
                Err(ManagedProcessError::Win32 {
                    operation: ManagedProcessOperation::AssignProcess,
                    source: io::Error::other("测试注入 Job 分配失败"),
                })
            });

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

    /// 验证完全替换环境按 ASCII 大小写不敏感顺序编码并以双 NUL 结束。
    #[test]
    fn builds_sorted_double_nul_unicode_environment_block() {
        let entries = BTreeMap::from([
            ("zeta".to_owned(), OsString::from("日本語 😀")),
            ("Alpha".to_owned(), OsString::from("中文")),
        ]);

        let block = build_unicode_environment_block(&entries).expect("合法替换环境应编码");
        let expected = "Alpha=中文\0zeta=日本語 😀\0\0"
            .encode_utf16()
            .collect::<Vec<_>>();

        assert_eq!(block, expected);
        assert_eq!(&block[block.len() - 2..], &[0, 0]);
        assert_eq!(
            build_unicode_environment_block(&BTreeMap::new()).expect("空环境应编码"),
            vec![0, 0]
        );
    }

    /// 验证大小写冲突、非 ASCII 键、NUL 与超限环境块在 CreateProcessW 前拒绝。
    #[test]
    fn rejects_invalid_or_oversized_replacement_environment() {
        let cases = [
            BTreeMap::from([
                ("Path".to_owned(), OsString::from("one")),
                ("PATH".to_owned(), OsString::from("two")),
            ]),
            BTreeMap::from([("中文".to_owned(), OsString::from("value"))]),
            BTreeMap::from([("VALID".to_owned(), OsString::from("nul\0value"))]),
            BTreeMap::from([("VALID".to_owned(), OsString::from("x".repeat(32_767)))]),
        ];

        for entries in cases {
            assert!(matches!(
                build_unicode_environment_block(&entries),
                Err(ManagedProcessError::InvalidEnvironment)
            ));
        }
    }

    /// 验证含空格和 CMD 元字符的 Artifact 路径通过 launch-only 环境安全执行。
    #[test]
    fn runs_cmd_with_unicode_values_and_special_artifact_path() {
        let root = std::env::temp_dir().join("CmdBox").join(format!(
            "special CMD 空格 & % ^ ! ( ) ' {}",
            uuid::Uuid::new_v4()
        ));
        let value = "中文 日本語 😀 space ' \" & % ^ ! ( ) < > | \\\\ tail";
        let script = format!(
            "@\"!{CMD_CHCP_ENVIRONMENT_NAME}!\" 65001 >nul\r\n@setlocal EnableExtensions EnableDelayedExpansion\r\n@echo off\r\necho(\r\necho(!CMDBOX_INTERNAL_VALUE_00000000!\r\n>&2 echo(stderr-日本語 😀\r\n@exit /b 7\r\n"
        );
        let artifact =
            MaterializedScript::create_in_root_for_test(RenderedScript::cmd(&script), root.clone())
                .expect("应在特殊路径创建 CMD Artifact");
        let runner = CmdRunner::resolve().expect("系统应提供 CMD 与 chcp");
        let launch = runner.process_launch_with_environment(
            artifact,
            &std::env::temp_dir(),
            BTreeMap::from([(
                "CMDBOX_INTERNAL_VALUE_00000000".to_owned(),
                OsString::from(value),
            )]),
        );
        let mut prepared = ManagedProcess::prepare(launch).expect("应准备受管 CMD");
        let (mut stdout, mut stderr) = prepared
            .take_output()
            .expect("应取得 CMD 输出 Pipe")
            .into_readers();
        let process = prepared.resume().expect("应恢复受管 CMD");

        let exit_code = process.wait().expect("CMD 应返回固定退出码");
        process.terminate_job().expect("退出后应清理 Job");
        process.wait_job_empty().expect("CMD Job 应为空");
        let mut stdout_text = String::new();
        let mut stderr_text = String::new();
        stdout
            .read_to_string(&mut stdout_text)
            .expect("stdout 应为 UTF-8");
        stderr
            .read_to_string(&mut stderr_text)
            .expect("stderr 应为 UTF-8");
        drop(process);

        assert_eq!(
            exit_code, 7,
            "CMD stderr={stderr_text:?}, stdout={stdout_text:?}"
        );
        assert_eq!(stdout_text, format!("\r\n{value}\r\n"));
        assert_eq!(stderr_text, "stderr-日本語 😀\r\n");
        assert!(root.is_dir(), "测试专属 Artifact 根应存在");
        fs::remove_dir(&root).expect("唯一 Execution 目录清理后根目录应为空");
        assert!(!root.exists(), "测试结束不得遗留 Artifact 根");
    }
}
