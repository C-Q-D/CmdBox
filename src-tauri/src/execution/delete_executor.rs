//! Windows 永久删除 Executor 的一次性 Named Pipe 授权与结果收集器。
//!
//! 本模块在 PowerShell 恢复运行前建立单实例、拒绝远程客户端的 Pipe，并要求调用方先绑定
//! 已创建但仍挂起的 PowerShell PID。客户端只有在 PID、一次性 token、generation、协议版本
//! 以及目标根最新 `PathFingerprint` 全部一致后才会收到 `APPROVE`。模块不执行删除，只接收
//! 每个目标唯一一次的 `SUCCESS` 或 `FAILURE` 事实；所有 I/O 都有边界、deadline 和取消路径。

#![allow(dead_code)] // CMD04-SESSION-01 接入前，生产构建尚没有启动此完整深模块的调用方。

use std::collections::BTreeMap;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::{Display, Formatter};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU32, Ordering};
#[cfg(test)]
use std::sync::Barrier;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use uuid::Uuid;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_NOT_FOUND,
    ERROR_OPERATION_ABORTED, ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED, HANDLE,
    INVALID_HANDLE_VALUE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::{
    ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{CreateEventW, SetEvent, WaitForMultipleObjects};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

use super::safety::{
    revalidate_delete_target, DeleteSafetyErrorCode, PathFingerprint, ProtectedPathSet,
};
use crate::execution::artifact::{ArtifactError, MaterializedScript, RenderedScript};
use crate::execution::outcome::OutcomePolicy;
use crate::process::windows::managed_process::{
    CapturedOutput, ManagedProcess, ManagedProcessCancellation, ManagedProcessError,
    PreparedManagedProcess,
};
use crate::process::windows::runner::ResolvedRunner;

/// 当前 collector 唯一接受的稳定协议版本。
const COLLECTOR_PROTOCOL_VERSION: u32 = 1;
/// Pipe 单方向内核缓冲区大小；协议消息仍受更小的行边界限制。
const PIPE_BUFFER_BYTES: u32 = 64 * 1024;
/// 一条 UTF-8 协议行允许的最大字节数，包含内容但不包含换行符。
const MAX_LINE_BYTES: usize = 512;
/// 单次 collector 会话允许读取的最大总字节数。
const MAX_SESSION_BYTES: usize = 256 * 1024;
/// Pipe 客户端和 PID 绑定必须在此时间内出现。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// 已连接客户端必须在此时间内完成 BEGIN 握手。
const BEGIN_TIMEOUT: Duration = Duration::from_secs(10);
/// 最终删除事实的总 deadline；取消仍可立即解除等待。
const FACTS_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
/// 服务端短响应的写入 deadline。
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// 永久删除深模块持有的完整、尚未物化执行计划。
pub(crate) struct DeleteExecutionPlan {
    /// Planner 已冻结并进入可信 Hash 的最终固定脚本。
    rendered_script: RenderedScript,
    /// 只从 Windows 系统目录解析的固定 Runner。
    runner: ResolvedRunner,
    /// 已由 Planner 验证且进入 Execution Spec 的工作目录。
    working_directory: PathBuf,
    /// 已进入 Execution Spec 的受限环境覆盖。
    environment: BTreeMap<String, OsString>,
    /// 随运行中进程交回 Session 的目标结果策略。
    outcome_policy: OutcomePolicy,
    /// Preview 与 Run 已绑定、必须在每个副作用前复验的目标身份。
    path_fingerprints: Vec<PathFingerprint>,
    /// 已进入 Execution Spec 的 collector 逻辑协议版本。
    protocol_version: u32,
}

impl DeleteExecutionPlan {
    /// 组合 Planner 已授权的删除材料；本方法不创建文件、Pipe 或进程。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        rendered_script: RenderedScript,
        runner: ResolvedRunner,
        working_directory: PathBuf,
        environment: BTreeMap<String, OsString>,
        outcome_policy: OutcomePolicy,
        path_fingerprints: Vec<PathFingerprint>,
        protocol_version: u32,
    ) -> Self {
        Self {
            rendered_script,
            runner,
            working_directory,
            environment,
            outcome_policy,
            path_fingerprints,
            protocol_version,
        }
    }

    /// 按 endpoint、Artifact、launch、挂起进程、PID 绑定的固定顺序准备执行。
    ///
    /// 任一步失败时，局部 RAII 所有者会取消 collector、删除 Artifact，并终止仍挂起的进程；
    /// 本方法绝不恢复主线程。
    pub(crate) fn prepare(self) -> Result<PreparedDeleteExecution, DeleteExecutorError> {
        let Self {
            rendered_script,
            runner,
            working_directory,
            environment,
            outcome_policy,
            path_fingerprints,
            protocol_version,
        } = self;

        let (collector_args, collector) =
            prepare_delete_collector(path_fingerprints, protocol_version)?;
        #[cfg(test)]
        let transport_pipe_leaf = collector_args.pipe_leaf.clone();
        let materialized_script = MaterializedScript::create(rendered_script)?;
        let launch = runner.process_launch_with_environment_and_arguments(
            materialized_script,
            &working_directory,
            environment,
            vec![
                OsString::from(&collector_args.pipe_leaf),
                OsString::from(&collector_args.token),
                OsString::from(&collector_args.generation),
            ],
        );
        #[cfg(test)]
        let temporary_directory = launch.temporary_directory().to_path_buf();
        let process = ManagedProcess::prepare(launch)?;
        collector.bind_expected_client_pid(process.process_id())?;

        Ok(PreparedDeleteExecution {
            process,
            collector,
            outcome_policy,
            #[cfg(test)]
            temporary_directory,
            #[cfg(test)]
            transport_pipe_leaf,
        })
    }
}

/// 删除 Executor 的稳定失败封装；格式化时不回显 Artifact、工作目录或目标路径。
pub(crate) enum DeleteExecutorError {
    /// 创建或复验受管脚本失败。
    Artifact(ArtifactError),
    /// 创建、挂起、Resume 或 Job 操作失败。
    ManagedProcess(ManagedProcessError),
    /// collector endpoint、身份授权或协议失败。
    Collector(DeleteCollectorError),
}

impl std::fmt::Debug for DeleteExecutorError {
    /// Debug 同样只输出稳定类别，避免上层调试日志间接泄露本机路径。
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, formatter)
    }
}

impl Display for DeleteExecutorError {
    /// 输出不包含底层路径或随机 transport secret 的稳定类别。
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Artifact(_) => "deleteExecutorArtifact",
            Self::ManagedProcess(_) => "deleteExecutorManagedProcess",
            Self::Collector(_) => "deleteExecutorCollector",
        })
    }
}

impl Error for DeleteExecutorError {}

impl From<ArtifactError> for DeleteExecutorError {
    /// 保留 Artifact 原始类型供内部匹配，但不让 Display 泄露路径。
    fn from(source: ArtifactError) -> Self {
        Self::Artifact(source)
    }
}

impl From<ManagedProcessError> for DeleteExecutorError {
    /// 包装受管进程失败并保持统一 Executor 边界。
    fn from(source: ManagedProcessError) -> Self {
        Self::ManagedProcess(source)
    }
}

impl From<DeleteCollectorError> for DeleteExecutorError {
    /// 包装 collector 失败并保持统一 Executor 边界。
    fn from(source: DeleteCollectorError) -> Self {
        Self::Collector(source)
    }
}

/// 已挂起创建、加入 Job 且完成 collector PID 绑定的删除执行。
pub(crate) struct PreparedDeleteExecution {
    /// 尚未 Resume 的 Job 受管 PowerShell。
    process: PreparedManagedProcess,
    /// 已与 process PID 绑定的 collector 租约。
    collector: DeleteCollectorLease,
    /// 运行结束后解释逐目标事实的版本化策略。
    outcome_policy: OutcomePolicy,
    /// 测试观察的受管 Artifact 唯一目录，不进入生产结构。
    #[cfg(test)]
    temporary_directory: PathBuf,
    /// 测试观察的一次性 Pipe 叶子名，不进入生产结构。
    #[cfg(test)]
    transport_pipe_leaf: String,
}

impl PreparedDeleteExecution {
    /// 返回挂起 PowerShell PID；不能据此提供任意 PID 终止能力。
    pub(crate) fn process_id(&self) -> u32 {
        self.process.process_id()
    }

    /// 取出 stdout/stderr 读端；调用方应在 Resume 前启动 drain。
    pub(crate) fn take_output(&mut self) -> Option<CapturedOutput> {
        self.process.take_output()
    }

    /// 返回同时终止 Job 并唤醒 collector 的可 Clone 窄取消入口。
    pub(crate) fn cancellation(&self) -> DeleteExecutionCancellation {
        DeleteExecutionCancellation {
            process: self.process.cancellation(),
            collector: self.collector.cancellation(),
        }
    }

    /// Resume 已加入 Job 且完成 PID 绑定的 PowerShell，并转移所有运行期租约。
    pub(crate) fn resume(self) -> Result<RunningDeleteExecution, DeleteExecutorError> {
        let Self {
            process,
            collector,
            outcome_policy,
            #[cfg(test)]
                temporary_directory: _,
            #[cfg(test)]
                transport_pipe_leaf: _,
        } = self;
        let process = process.resume()?;
        Ok(RunningDeleteExecution {
            process,
            collector,
            outcome_policy,
        })
    }

    /// 返回当前受管 Artifact 唯一目录，仅供真实模块集成测试检查清理。
    #[cfg(test)]
    fn temporary_directory(&self) -> &std::path::Path {
        &self.temporary_directory
    }

    /// 返回当前一次性 Pipe 叶子名，仅供测试确认 watcher 退出后不再可连接。
    #[cfg(test)]
    fn transport_pipe_leaf(&self) -> &str {
        &self.transport_pipe_leaf
    }
}

/// 同时覆盖 PowerShell Job 与 collector watcher 的可 Clone 取消能力。
#[derive(Clone)]
pub(crate) struct DeleteExecutionCancellation {
    /// 与运行中 PowerShell 共享的 KILL_ON_JOB_CLOSE Job 入口。
    process: ManagedProcessCancellation,
    /// 唤醒 collector 所有 pending I/O 的事件入口。
    collector: DeleteCollectorCancellation,
}

impl DeleteExecutionCancellation {
    /// 先唤醒 collector，再请求终止整个 Job；Job 失败也不会留下阻塞 watcher。
    pub(crate) fn cancel(&self) -> Result<(), DeleteExecutorError> {
        self.collector.cancel();
        self.process.terminate_job()?;
        Ok(())
    }
}

/// 已 Resume 的删除执行及其完整运行期所有权。
pub(crate) struct RunningDeleteExecution {
    /// 可等待、可终止且受 Job 管理的 PowerShell 根进程。
    process: ManagedProcess,
    /// 收集逐目标事实并持有 Pipe 生命周期的租约。
    collector: DeleteCollectorLease,
    /// 只在完整自然终态后消费 collector 事实的策略。
    outcome_policy: OutcomePolicy,
}

impl RunningDeleteExecution {
    /// 将三个受信运行期部分一次性交给 Session 等待与结果解释路径。
    pub(crate) fn into_parts(self) -> (ManagedProcess, DeleteCollectorLease, OutcomePolicy) {
        (self.process, self.collector, self.outcome_policy)
    }
}

/// 可供固定 PowerShell 模板使用的 launch-local collector 参数。
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct DeleteCollectorLaunchArgs {
    /// 传给 `NamedPipeClientStream` 的 Pipe 叶子名，不含 `\\.\pipe\` 前缀。
    pub(crate) pipe_leaf: String,
    /// 每次启动新生成且只接受一次的高熵握手 token。
    pub(crate) token: String,
    /// 区分同一 Execution 不同启动代际的随机 generation。
    pub(crate) generation: String,
    /// 固定脚本与服务端共同验证的 collector 协议版本。
    pub(crate) protocol_version: u32,
}

/// 一个目标在受信 PowerShell 客户端中观察到的最终事实。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DeleteTargetFact {
    /// 目标对应删除动作成功。
    Success {
        /// 折叠后目标在 Canonical Execution Spec 中的序号。
        target_index: usize,
    },
    /// 目标对应删除动作失败。
    Failure {
        /// 折叠后目标在 Canonical Execution Spec 中的序号。
        target_index: usize,
        /// 固定脚本给出的受限机器错误码，不接受任意文本。
        error_code: String,
    },
}

/// collector 完整收到并确认的有序逐目标事实。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeleteCollectorResult {
    /// 按目标序号排列且每个目标恰好一项的终态事实。
    pub(crate) target_facts: Vec<DeleteTargetFact>,
}

/// collector 可稳定分类的失败原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeleteCollectorErrorCode {
    /// 调用方请求了尚未实现的协议版本。
    UnsupportedProtocol,
    /// Windows 无法创建单实例 Pipe。
    CreatePipe,
    /// Windows 无法创建取消或 Overlapped 事件。
    CreateEvent,
    /// watcher 线程无法启动。
    SpawnWatcher,
    /// 调用方传入空的目标集合。
    EmptyTargets,
    /// 调用方在恢复客户端前没有绑定有效 PID。
    ClientPidUnbound,
    /// 预期客户端 PID 已经绑定，不能再次改变。
    ClientPidAlreadyBound,
    /// 连接 Pipe 的进程不是 Session 刚创建的 PowerShell。
    ClientPidMismatch,
    /// 目标在 BEGIN 前发生变化或最新 Safety Guard 不再通过。
    SafetyChanged,
    /// watcher 被 Session 或 Drop 主动取消。
    Cancelled,
    /// 连接、握手、写入或结果收集超过对应 deadline。
    Timeout,
    /// 客户端在完整终态协议前断开。
    Disconnected,
    /// Windows Pipe I/O 返回其他失败。
    Io,
    /// 输入不是合法的无 NUL UTF-8 行。
    InvalidEncoding,
    /// 单行或会话总输入超过固定上限。
    InputTooLarge,
    /// 客户端消息不符合 v1 状态机或字段约束。
    InvalidProtocol,
    /// watcher 线程发生 panic，不能伪造业务结果。
    WatcherPanicked,
}

impl DeleteCollectorErrorCode {
    /// 返回不依赖 Debug 格式的内部稳定错误码。
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedProtocol => "unsupportedProtocol",
            Self::CreatePipe => "createPipe",
            Self::CreateEvent => "createEvent",
            Self::SpawnWatcher => "spawnWatcher",
            Self::EmptyTargets => "emptyTargets",
            Self::ClientPidUnbound => "clientPidUnbound",
            Self::ClientPidAlreadyBound => "clientPidAlreadyBound",
            Self::ClientPidMismatch => "clientPidMismatch",
            Self::SafetyChanged => "safetyChanged",
            Self::Cancelled => "cancelled",
            Self::Timeout => "timeout",
            Self::Disconnected => "disconnected",
            Self::Io => "io",
            Self::InvalidEncoding => "invalidEncoding",
            Self::InputTooLarge => "inputTooLarge",
            Self::InvalidProtocol => "invalidProtocol",
            Self::WatcherPanicked => "watcherPanicked",
        }
    }
}

/// 不回显本机路径或任意客户端文本的 collector 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeleteCollectorError {
    /// 供 Session、Outcome 和测试稳定分支处理的原因。
    pub(crate) code: DeleteCollectorErrorCode,
}

impl DeleteCollectorError {
    /// 创建不包含本机敏感信息的稳定错误。
    const fn new(code: DeleteCollectorErrorCode) -> Self {
        Self { code }
    }
}

impl Display for DeleteCollectorError {
    /// 只输出稳定错误码，避免把 Pipe 名、token 或路径写入日志。
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code.as_str())
    }
}

impl Error for DeleteCollectorError {}

/// cancel 与 APPROVE 写入共享的线性化状态。
enum ApprovalGateState {
    /// 当前 Execution 尚可为 fresh 目标发出授权。
    Active,
    /// cancel 已经线性化，后续任何目标都不得再收到授权。
    Cancelled,
}

/// watcher 内部测试暂停点；生产构建为空且没有同步开销。
#[derive(Clone, Default)]
struct CollectorTestHooks {
    /// fresh guard 通过后、获取 approval gate 前的确定性暂停点。
    #[cfg(test)]
    before_approval_gate: Option<TestPause>,
    /// 首次 Overlapped ReadFile 已 pending、进入 wait 前的确定性暂停点。
    #[cfg(test)]
    pending_read: Option<TestPause>,
}

/// 由测试线程和 watcher 共同控制的双 Barrier 暂停点。
#[cfg(test)]
#[derive(Clone)]
struct TestPause {
    /// watcher 到达目标位置时通知测试线程。
    reached: Arc<Barrier>,
    /// 测试完成并发动作后允许 watcher 继续。
    release: Arc<Barrier>,
}

#[cfg(test)]
impl TestPause {
    /// 创建一个只供一个 watcher 和一个测试线程配对的暂停点。
    fn new() -> Self {
        Self {
            reached: Arc::new(Barrier::new(2)),
            release: Arc::new(Barrier::new(2)),
        }
    }

    /// watcher 报告已到达并等待测试线程释放。
    fn pause_watcher(&self) {
        self.reached.wait();
        self.release.wait();
    }

    /// 测试线程等待 watcher 到达。
    fn wait_until_reached(&self) {
        self.reached.wait();
    }

    /// 测试线程释放 watcher。
    fn release_watcher(&self) {
        self.release.wait();
    }
}

/// 可在线程间 Clone、只能触发当前 collector 取消事件的窄句柄。
#[derive(Clone)]
pub(crate) struct DeleteCollectorCancellation {
    /// 与租约共享但不拥有 watcher join 权限的 manual-reset 事件。
    cancel_event: Arc<OwnedKernelHandle>,
    /// 与所有 APPROVE 写入串行化的共享授权门。
    approval_gate: Arc<Mutex<ApprovalGateState>>,
}

impl DeleteCollectorCancellation {
    /// 线性化关闭授权门，再唤醒当前 collector 的 Connect、Read 或 Write。
    ///
    /// 方法返回前会等待任何已经持锁的 APPROVE 写入结束；返回后不存在在途授权写入。
    pub(crate) fn cancel(&self) {
        let mut state = self
            .approval_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *state = ApprovalGateState::Cancelled;
        // SAFETY: Arc 保证 event 在调用期间有效，manual-reset event 可重复 SetEvent。
        unsafe { SetEvent(self.cancel_event.raw()) };
    }
}

/// 创建完成后由 Session 持有的 collector 生命周期租约。
pub(crate) struct DeleteCollectorLease {
    /// Drop 和显式取消共同触发的 manual-reset 事件。
    cancel_event: Arc<OwnedKernelHandle>,
    /// PID 绑定完成后唤醒 watcher 的 manual-reset 事件。
    pid_bound_event: Arc<OwnedKernelHandle>,
    /// cancel 与 watcher APPROVE 写入共享的线性化门。
    approval_gate: Arc<Mutex<ApprovalGateState>>,
    /// 只允许从零设置一次的预期 PowerShell PID。
    expected_client_pid: Arc<AtomicU32>,
    /// 独占 Pipe 和所有 Overlapped 状态的 watcher。
    watcher: Option<JoinHandle<Result<DeleteCollectorResult, DeleteCollectorError>>>,
}

impl DeleteCollectorLease {
    /// 在挂起的 PowerShell 恢复前一次性绑定其 PID；零值和重复绑定都会失败。
    pub(crate) fn bind_expected_client_pid(
        &self,
        process_id: u32,
    ) -> Result<(), DeleteCollectorError> {
        if process_id == 0 {
            return Err(DeleteCollectorError::new(
                DeleteCollectorErrorCode::ClientPidUnbound,
            ));
        }
        self.expected_client_pid
            .compare_exchange(0, process_id, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                DeleteCollectorError::new(DeleteCollectorErrorCode::ClientPidAlreadyBound)
            })?;
        if unsafe { SetEvent(self.pid_bound_event.raw()) } == 0 {
            self.cancel();
            return Err(DeleteCollectorError::new(
                DeleteCollectorErrorCode::CreateEvent,
            ));
        }
        Ok(())
    }

    /// 请求 watcher 取消当前 Connect、Read 或 Write；方法幂等且不等待线程退出。
    pub(crate) fn cancel(&self) {
        self.cancellation().cancel();
    }

    /// 返回不含 join、finish 或 Pipe 所有权的可 Clone 取消入口。
    pub(crate) fn cancellation(&self) -> DeleteCollectorCancellation {
        DeleteCollectorCancellation {
            cancel_event: Arc::clone(&self.cancel_event),
            approval_gate: Arc::clone(&self.approval_gate),
        }
    }

    /// 等待 collector 得到完整逐目标事实；线程 panic 必须按内部失败处理。
    pub(crate) fn finish(mut self) -> Result<DeleteCollectorResult, DeleteCollectorError> {
        self.join_watcher()
    }

    /// 只消费一次 watcher 句柄并映射 panic。
    fn join_watcher(&mut self) -> Result<DeleteCollectorResult, DeleteCollectorError> {
        let Some(watcher) = self.watcher.take() else {
            return Err(DeleteCollectorError::new(
                DeleteCollectorErrorCode::WatcherPanicked,
            ));
        };
        watcher.join().unwrap_or_else(|_| {
            Err(DeleteCollectorError::new(
                DeleteCollectorErrorCode::WatcherPanicked,
            ))
        })
    }
}

impl Drop for DeleteCollectorLease {
    /// 解除任何 pending I/O 并等待 watcher 排空 OVERLAPPED 后再释放共享事件。
    fn drop(&mut self) {
        if self.watcher.is_some() {
            self.cancel();
            let _ = self.join_watcher();
        }
    }
}

/// 建立 Pipe、生成 launch-local 参数并启动 watcher；本函数不会运行或删除任何目标。
pub(crate) fn prepare_delete_collector(
    expected_fingerprints: Vec<PathFingerprint>,
    protocol_version: u32,
) -> Result<(DeleteCollectorLaunchArgs, DeleteCollectorLease), DeleteCollectorError> {
    prepare_delete_collector_internal(
        expected_fingerprints,
        protocol_version,
        CollectorTestHooks::default(),
    )
}

/// 测试专用入口，为 watcher 注入确定性并发暂停点。
#[cfg(test)]
fn prepare_delete_collector_with_hooks(
    expected_fingerprints: Vec<PathFingerprint>,
    protocol_version: u32,
    hooks: CollectorTestHooks,
) -> Result<(DeleteCollectorLaunchArgs, DeleteCollectorLease), DeleteCollectorError> {
    prepare_delete_collector_internal(expected_fingerprints, protocol_version, hooks)
}

/// 建立 collector 的共享实现；生产入口始终传入空 hook。
fn prepare_delete_collector_internal(
    expected_fingerprints: Vec<PathFingerprint>,
    protocol_version: u32,
    hooks: CollectorTestHooks,
) -> Result<(DeleteCollectorLaunchArgs, DeleteCollectorLease), DeleteCollectorError> {
    if protocol_version != COLLECTOR_PROTOCOL_VERSION {
        return Err(DeleteCollectorError::new(
            DeleteCollectorErrorCode::UnsupportedProtocol,
        ));
    }
    if expected_fingerprints.is_empty() {
        return Err(DeleteCollectorError::new(
            DeleteCollectorErrorCode::EmptyTargets,
        ));
    }

    let generation = Uuid::new_v4().to_string();
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let pipe_leaf = format!("cmdbox-delete-{generation}-{}", Uuid::new_v4().simple());
    let full_pipe_name = format!(r"\\.\pipe\{pipe_leaf}");
    let full_pipe_name = wide_null(OsStr::new(&full_pipe_name));

    // SAFETY: 名称是 NUL 结尾 UTF-16；Security Attributes 为空时使用进程默认 DACL。
    let pipe = unsafe {
        CreateNamedPipeW(
            full_pipe_name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            PIPE_BUFFER_BYTES,
            PIPE_BUFFER_BYTES,
            0,
            null(),
        )
    };
    if pipe == INVALID_HANDLE_VALUE {
        return Err(DeleteCollectorError::new(
            DeleteCollectorErrorCode::CreatePipe,
        ));
    }
    let pipe = OwnedKernelHandle(pipe);
    let cancel_event = Arc::new(create_manual_event()?);
    let pid_bound_event = Arc::new(create_manual_event()?);
    let approval_gate = Arc::new(Mutex::new(ApprovalGateState::Active));
    let expected_client_pid = Arc::new(AtomicU32::new(0));

    let watcher_cancel = Arc::clone(&cancel_event);
    let watcher_pid_event = Arc::clone(&pid_bound_event);
    let watcher_expected_pid = Arc::clone(&expected_client_pid);
    let watcher_approval_gate = Arc::clone(&approval_gate);
    let watcher_token = token.clone();
    let watcher_generation = generation.clone();
    let watcher = thread::Builder::new()
        .name("cmdbox-delete-collector".to_owned())
        .spawn(move || {
            run_watcher(
                pipe,
                watcher_cancel,
                watcher_pid_event,
                watcher_expected_pid,
                watcher_approval_gate,
                expected_fingerprints,
                watcher_generation,
                watcher_token,
                hooks,
            )
        })
        .map_err(|_| DeleteCollectorError::new(DeleteCollectorErrorCode::SpawnWatcher))?;

    Ok((
        DeleteCollectorLaunchArgs {
            pipe_leaf,
            token,
            generation,
            protocol_version,
        },
        DeleteCollectorLease {
            cancel_event,
            pid_bound_event,
            approval_gate,
            expected_client_pid,
            watcher: Some(watcher),
        },
    ))
}

/// watcher 的完整连接、身份授权与结果收集状态机。
#[allow(clippy::too_many_arguments)]
fn run_watcher(
    pipe: OwnedKernelHandle,
    cancel_event: Arc<OwnedKernelHandle>,
    pid_bound_event: Arc<OwnedKernelHandle>,
    expected_client_pid: Arc<AtomicU32>,
    approval_gate: Arc<Mutex<ApprovalGateState>>,
    expected_fingerprints: Vec<PathFingerprint>,
    generation: String,
    token: String,
    hooks: CollectorTestHooks,
) -> Result<DeleteCollectorResult, DeleteCollectorError> {
    #[cfg(not(test))]
    let _ = hooks;
    let connect_deadline = Instant::now() + CONNECT_TIMEOUT;
    connect_overlapped(pipe.raw(), cancel_event.raw(), connect_deadline)?;
    wait_until_pid_bound(pid_bound_event.raw(), cancel_event.raw(), connect_deadline)?;
    verify_client_pid(pipe.raw(), expected_client_pid.load(Ordering::Acquire))?;

    let mut reader = BoundedLineReader::new(
        #[cfg(test)]
        hooks.pending_read.clone(),
    );
    let facts_deadline = Instant::now() + FACTS_TIMEOUT;
    let mut target_facts = Vec::with_capacity(expected_fingerprints.len());
    for (target_index, expected) in expected_fingerprints.iter().enumerate() {
        let begin_deadline = if target_index == 0 {
            Instant::now() + BEGIN_TIMEOUT
        } else {
            facts_deadline
        };
        let begin = reader.read_line(pipe.raw(), cancel_event.raw(), begin_deadline)?;
        if reader.has_buffered_input() || !valid_begin(&begin, &token, &generation, target_index) {
            let _ = write_protocol_line(
                pipe.raw(),
                cancel_event.raw(),
                "DENY|invalidProtocol",
                Instant::now() + WRITE_TIMEOUT,
            );
            return Err(DeleteCollectorError::new(
                DeleteCollectorErrorCode::InvalidProtocol,
            ));
        }

        let protected_paths = ProtectedPathSet::for_cmdbox()
            .map_err(|_| DeleteCollectorError::new(DeleteCollectorErrorCode::SafetyChanged))?;
        if revalidate_delete_target(target_index, expected, &protected_paths).is_err() {
            let _ = write_protocol_line(
                pipe.raw(),
                cancel_event.raw(),
                "DENY|safetyChanged",
                Instant::now() + WRITE_TIMEOUT,
            );
            return Err(DeleteCollectorError::new(
                DeleteCollectorErrorCode::SafetyChanged,
            ));
        }

        #[cfg(test)]
        if let Some(hook) = &hooks.before_approval_gate {
            hook.pause_watcher();
        }
        write_approval_under_gate(
            &approval_gate,
            pipe.raw(),
            cancel_event.raw(),
            &format!("APPROVE|{token}|{generation}|{target_index}"),
        )?;
        let fact_line = reader.read_line(pipe.raw(), cancel_event.raw(), facts_deadline)?;
        let fact = parse_fact(
            &fact_line,
            &token,
            &generation,
            target_index,
            expected,
            &protected_paths,
        )?;
        target_facts.push(fact);
        write_protocol_line(
            pipe.raw(),
            cancel_event.raw(),
            &format!("ACK|{token}|{generation}|{target_index}"),
            Instant::now() + WRITE_TIMEOUT,
        )?;
    }
    if reader.has_buffered_input() {
        return Err(DeleteCollectorError::new(
            DeleteCollectorErrorCode::InvalidProtocol,
        ));
    }

    // 让 RAII CloseHandle 结束一次性实例；显式 DisconnectNamedPipe 会丢弃客户端尚未读取的 ACK。
    Ok(DeleteCollectorResult { target_facts })
}

/// 在共享 gate 内检查取消状态并完成整个 APPROVE 写入，使授权与 cancel 线性化。
fn write_approval_under_gate(
    approval_gate: &Mutex<ApprovalGateState>,
    pipe: HANDLE,
    cancel_event: HANDLE,
    approval: &str,
) -> Result<(), DeleteCollectorError> {
    let state = approval_gate
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if matches!(*state, ApprovalGateState::Cancelled) {
        return Err(DeleteCollectorError::new(
            DeleteCollectorErrorCode::Cancelled,
        ));
    }
    write_protocol_line(pipe, cancel_event, approval, Instant::now() + WRITE_TIMEOUT)
}

/// 等待调用方在恢复客户端前发布预期 PID。
fn wait_until_pid_bound(
    pid_bound_event: HANDLE,
    cancel_event: HANDLE,
    deadline: Instant,
) -> Result<(), DeleteCollectorError> {
    match wait_for_signal(pid_bound_event, cancel_event, deadline)? {
        WaitSignal::Operation => Ok(()),
        WaitSignal::Cancelled => Err(DeleteCollectorError::new(
            DeleteCollectorErrorCode::Cancelled,
        )),
    }
}

/// 比较实际 Pipe 客户端 PID 与 Session 在 Resume 前绑定的唯一 PID。
fn verify_client_pid(pipe: HANDLE, expected_pid: u32) -> Result<(), DeleteCollectorError> {
    if expected_pid == 0 {
        return Err(DeleteCollectorError::new(
            DeleteCollectorErrorCode::ClientPidUnbound,
        ));
    }
    let mut actual_pid = 0_u32;
    // SAFETY: pipe 已连接；actual_pid 是有效可写指针。
    if unsafe { GetNamedPipeClientProcessId(pipe, &mut actual_pid) } == 0 {
        return Err(DeleteCollectorError::new(DeleteCollectorErrorCode::Io));
    }
    if actual_pid != expected_pid {
        return Err(DeleteCollectorError::new(
            DeleteCollectorErrorCode::ClientPidMismatch,
        ));
    }
    Ok(())
}

/// 严格验证固定脚本逐目标 BEGIN 的字段数量、token、generation 和顺序 index。
fn valid_begin(line: &str, token: &str, generation: &str, target_index: usize) -> bool {
    let fields = line.split('|').collect::<Vec<_>>();
    let expected_index = target_index.to_string();
    matches!(
        fields.as_slice(),
        ["BEGIN", actual_token, actual_generation, actual_index]
            if *actual_token == token
                && *actual_generation == generation
                && *actual_index == expected_index
    )
}

/// 解析固定脚本的单目标事实；SUCCESS 还必须由 Rust 观察到目标根确实 NotFound。
fn parse_fact(
    line: &str,
    token: &str,
    generation: &str,
    target_index: usize,
    expected: &PathFingerprint,
    protected_paths: &ProtectedPathSet,
) -> Result<DeleteTargetFact, DeleteCollectorError> {
    let fields = line.split('|').collect::<Vec<_>>();
    let expected_index = target_index.to_string();
    match fields.as_slice() {
        ["SUCCESS", actual_token, actual_generation, actual_index]
            if *actual_token == token
                && *actual_generation == generation
                && *actual_index == expected_index =>
        {
            match revalidate_delete_target(target_index, expected, protected_paths) {
                Err(error) if error.code == DeleteSafetyErrorCode::NotFound => {
                    Ok(DeleteTargetFact::Success { target_index })
                }
                _ => Err(DeleteCollectorError::new(
                    DeleteCollectorErrorCode::InvalidProtocol,
                )),
            }
        }
        ["FAILURE", actual_token, actual_generation, actual_index, error_code]
            if *actual_token == token
                && *actual_generation == generation
                && *actual_index == expected_index
                && matches!(*error_code, "stillExists" | "removeFailed") =>
        {
            Ok(DeleteTargetFact::Failure {
                target_index,
                error_code: (*error_code).to_owned(),
            })
        }
        _ => Err(DeleteCollectorError::new(
            DeleteCollectorErrorCode::InvalidProtocol,
        )),
    }
}

/// 使用 OVERLAPPED ConnectNamedPipe，并处理客户端抢先连接的合法竞争窗口。
fn connect_overlapped(
    pipe: HANDLE,
    cancel_event: HANDLE,
    deadline: Instant,
) -> Result<(), DeleteCollectorError> {
    let operation_event = create_manual_event()?;
    let mut overlapped = OVERLAPPED {
        hEvent: operation_event.raw(),
        ..OVERLAPPED::default()
    };
    // SAFETY: pipe 以 FILE_FLAG_OVERLAPPED 创建；OVERLAPPED 和 event 活到 operation 完成。
    if unsafe { ConnectNamedPipe(pipe, &mut overlapped) } != 0 {
        return Ok(());
    }
    match unsafe { GetLastError() } {
        ERROR_PIPE_CONNECTED => Ok(()),
        ERROR_IO_PENDING => wait_pending_operation(
            pipe,
            &overlapped,
            operation_event.raw(),
            cancel_event,
            deadline,
        )
        .map(|_| ()),
        _ => Err(DeleteCollectorError::new(DeleteCollectorErrorCode::Io)),
    }
}

/// 一个严格有界的 byte-mode UTF-8 行读取器。
struct BoundedLineReader {
    /// 尚未消费的 Pipe 字节；可能包含下一条完整行。
    pending: Vec<u8>,
    /// 当前 session 已从内核读出的累计字节数。
    total_read: usize,
    /// 首次 pending ReadFile 的测试暂停点。
    #[cfg(test)]
    pending_read_hook: Option<TestPause>,
    /// 保证 pending ReadFile 暂停点只触发一次。
    #[cfg(test)]
    pending_read_hook_used: bool,
}

impl BoundedLineReader {
    /// 创建空的有界读取状态。
    fn new(#[cfg(test)] pending_read_hook: Option<TestPause>) -> Self {
        Self {
            pending: Vec::new(),
            total_read: 0,
            #[cfg(test)]
            pending_read_hook,
            #[cfg(test)]
            pending_read_hook_used: false,
        }
    }

    /// 在绝对 deadline 前读取一条 `\n` 结尾的严格 UTF-8 行。
    fn read_line(
        &mut self,
        pipe: HANDLE,
        cancel_event: HANDLE,
        deadline: Instant,
    ) -> Result<String, DeleteCollectorError> {
        loop {
            if let Some(newline) = self.pending.iter().position(|byte| *byte == b'\n') {
                if newline > MAX_LINE_BYTES {
                    return Err(DeleteCollectorError::new(
                        DeleteCollectorErrorCode::InputTooLarge,
                    ));
                }
                let mut line = self.pending.drain(..=newline).collect::<Vec<_>>();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if line.contains(&0) {
                    return Err(DeleteCollectorError::new(
                        DeleteCollectorErrorCode::InvalidEncoding,
                    ));
                }
                return String::from_utf8(line).map_err(|_| {
                    DeleteCollectorError::new(DeleteCollectorErrorCode::InvalidEncoding)
                });
            }
            if self.pending.len() > MAX_LINE_BYTES {
                return Err(DeleteCollectorError::new(
                    DeleteCollectorErrorCode::InputTooLarge,
                ));
            }
            let mut chunk = [0_u8; 4096];
            #[cfg(test)]
            let pending_read_hook = if self.pending_read_hook_used {
                None
            } else {
                self.pending_read_hook.clone()
            };
            #[cfg(test)]
            if pending_read_hook.is_some() {
                self.pending_read_hook_used = true;
            }
            let read = read_overlapped(
                pipe,
                cancel_event,
                deadline,
                &mut chunk,
                #[cfg(test)]
                pending_read_hook.as_ref(),
            )?;
            if read == 0 {
                return Err(DeleteCollectorError::new(
                    DeleteCollectorErrorCode::Disconnected,
                ));
            }
            self.total_read = self.total_read.saturating_add(read);
            if self.total_read > MAX_SESSION_BYTES {
                return Err(DeleteCollectorError::new(
                    DeleteCollectorErrorCode::InputTooLarge,
                ));
            }
            self.pending.extend_from_slice(&chunk[..read]);
        }
    }

    /// 判断最后一个目标 ACK 前是否已经携带未消费的尾随输入。
    fn has_buffered_input(&self) -> bool {
        !self.pending.is_empty()
    }
}

/// 执行一次可取消、受 deadline 限制的 Overlapped ReadFile。
fn read_overlapped(
    pipe: HANDLE,
    cancel_event: HANDLE,
    deadline: Instant,
    buffer: &mut [u8],
    #[cfg(test)] pending_read_hook: Option<&TestPause>,
) -> Result<usize, DeleteCollectorError> {
    let operation_event = create_manual_event()?;
    let mut overlapped = OVERLAPPED {
        hEvent: operation_event.raw(),
        ..OVERLAPPED::default()
    };
    // SAFETY: buffer 和 OVERLAPPED 在 operation completion 被排空前保持有效。
    if unsafe {
        ReadFile(
            pipe,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            null_mut(),
            &mut overlapped,
        )
    } != 0
    {
        return completed_bytes(pipe, &overlapped).map(|value| value as usize);
    }
    let error = unsafe { GetLastError() };
    if matches!(error, ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED) {
        return Err(DeleteCollectorError::new(
            DeleteCollectorErrorCode::Disconnected,
        ));
    }
    if error != ERROR_IO_PENDING {
        return Err(DeleteCollectorError::new(DeleteCollectorErrorCode::Io));
    }
    #[cfg(test)]
    if let Some(hook) = pending_read_hook {
        hook.pause_watcher();
    }
    wait_pending_operation(
        pipe,
        &overlapped,
        operation_event.raw(),
        cancel_event,
        deadline,
    )
    .map(|value| value as usize)
}

/// 写出一条不含内嵌换行的有界协议行，并处理部分写入。
fn write_protocol_line(
    pipe: HANDLE,
    cancel_event: HANDLE,
    line: &str,
    deadline: Instant,
) -> Result<(), DeleteCollectorError> {
    if line.as_bytes().contains(&b'\n')
        || line.as_bytes().contains(&b'\r')
        || line.len() > MAX_LINE_BYTES
    {
        return Err(DeleteCollectorError::new(
            DeleteCollectorErrorCode::InvalidProtocol,
        ));
    }
    let mut bytes = line.as_bytes().to_vec();
    bytes.push(b'\n');
    let mut written = 0_usize;
    while written < bytes.len() {
        let count = write_overlapped(pipe, cancel_event, deadline, &bytes[written..])?;
        if count == 0 {
            return Err(DeleteCollectorError::new(
                DeleteCollectorErrorCode::Disconnected,
            ));
        }
        written += count;
    }
    Ok(())
}

/// 执行一次可取消、受 deadline 限制的 Overlapped WriteFile。
fn write_overlapped(
    pipe: HANDLE,
    cancel_event: HANDLE,
    deadline: Instant,
    buffer: &[u8],
) -> Result<usize, DeleteCollectorError> {
    let operation_event = create_manual_event()?;
    let mut overlapped = OVERLAPPED {
        hEvent: operation_event.raw(),
        ..OVERLAPPED::default()
    };
    // SAFETY: buffer 和 OVERLAPPED 在 operation completion 被排空前保持有效。
    if unsafe {
        WriteFile(
            pipe,
            buffer.as_ptr(),
            buffer.len() as u32,
            null_mut(),
            &mut overlapped,
        )
    } != 0
    {
        return completed_bytes(pipe, &overlapped).map(|value| value as usize);
    }
    let error = unsafe { GetLastError() };
    if matches!(error, ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED) {
        return Err(DeleteCollectorError::new(
            DeleteCollectorErrorCode::Disconnected,
        ));
    }
    if error != ERROR_IO_PENDING {
        return Err(DeleteCollectorError::new(DeleteCollectorErrorCode::Io));
    }
    wait_pending_operation(
        pipe,
        &overlapped,
        operation_event.raw(),
        cancel_event,
        deadline,
    )
    .map(|value| value as usize)
}

/// 等待一个 pending operation 或取消事件，并在取消/超时时先排空 completion。
fn wait_pending_operation(
    pipe: HANDLE,
    overlapped: &OVERLAPPED,
    operation_event: HANDLE,
    cancel_event: HANDLE,
    deadline: Instant,
) -> Result<u32, DeleteCollectorError> {
    match wait_for_signal(operation_event, cancel_event, deadline) {
        Ok(WaitSignal::Operation) => completed_bytes(pipe, overlapped),
        Ok(WaitSignal::Cancelled) => {
            cancel_and_drain(pipe, overlapped);
            Err(DeleteCollectorError::new(
                DeleteCollectorErrorCode::Cancelled,
            ))
        }
        Err(error) if error.code == DeleteCollectorErrorCode::Timeout => {
            cancel_and_drain(pipe, overlapped);
            Err(error)
        }
        Err(error) => {
            cancel_and_drain(pipe, overlapped);
            Err(error)
        }
    }
}

/// 取得已经完成的 Overlapped operation 字节数。
fn completed_bytes(pipe: HANDLE, overlapped: &OVERLAPPED) -> Result<u32, DeleteCollectorError> {
    let mut transferred = 0_u32;
    // SAFETY: 调用方只在 operation 已同步完成或其 event 已触发后调用，结构仍然有效。
    if unsafe { GetOverlappedResult(pipe, overlapped, &mut transferred, 0) } == 0 {
        let error = unsafe { GetLastError() };
        return Err(DeleteCollectorError::new(
            if matches!(error, ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED) {
                DeleteCollectorErrorCode::Disconnected
            } else if error == ERROR_OPERATION_ABORTED {
                DeleteCollectorErrorCode::Cancelled
            } else {
                DeleteCollectorErrorCode::Io
            },
        ));
    }
    Ok(transferred)
}

/// 取消指定 Overlapped operation，并在释放其栈内结构前等待内核不再访问。
fn cancel_and_drain(pipe: HANDLE, overlapped: &OVERLAPPED) {
    // SAFETY: pipe 和 OVERLAPPED 在本函数返回前保持有效；ERROR_NOT_FOUND 表示完成竞争已结束。
    let _cancelled = unsafe { CancelIoEx(pipe, overlapped) } != 0
        || unsafe { GetLastError() } == ERROR_NOT_FOUND;
    let mut transferred = 0_u32;
    // SAFETY: 即使取消与完成竞争，等待最终 completion 后才能释放 OVERLAPPED。
    unsafe { GetOverlappedResult(pipe, overlapped, &mut transferred, 1) };
}

/// 等待 operation/PID 事件或全局取消事件，使用同一个绝对 deadline。
fn wait_for_signal(
    operation_event: HANDLE,
    cancel_event: HANDLE,
    deadline: Instant,
) -> Result<WaitSignal, DeleteCollectorError> {
    let timeout = remaining_millis(deadline)
        .ok_or_else(|| DeleteCollectorError::new(DeleteCollectorErrorCode::Timeout))?;
    let handles = [operation_event, cancel_event];
    // SAFETY: 两个 event handle 在等待期间均由当前栈或 Arc 保持有效。
    match unsafe { WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, timeout) } {
        WAIT_OBJECT_0 => Ok(WaitSignal::Operation),
        value if value == WAIT_OBJECT_0 + 1 => Ok(WaitSignal::Cancelled),
        WAIT_TIMEOUT => Err(DeleteCollectorError::new(DeleteCollectorErrorCode::Timeout)),
        WAIT_FAILED => Err(DeleteCollectorError::new(DeleteCollectorErrorCode::Io)),
        _ => Err(DeleteCollectorError::new(DeleteCollectorErrorCode::Io)),
    }
}

/// 把剩余 duration 向上取整为 Win32 毫秒，避免亚毫秒 deadline 被当成立即超时。
fn remaining_millis(deadline: Instant) -> Option<u32> {
    let remaining = deadline.checked_duration_since(Instant::now())?;
    if remaining.is_zero() {
        return None;
    }
    Some(
        remaining
            .as_millis()
            .saturating_add(1)
            .min(u128::from(u32::MAX)) as u32,
    )
}

/// WaitForMultipleObjects 的两个合法事件分支。
enum WaitSignal {
    /// 当前 Overlapped operation 或 PID 绑定已完成。
    Operation,
    /// Session/Drop 请求取消。
    Cancelled,
}

/// 创建初始未触发的 manual-reset event。
fn create_manual_event() -> Result<OwnedKernelHandle, DeleteCollectorError> {
    // SAFETY: 使用默认 Security Attributes、manual-reset、初始未触发且无名称。
    let handle = unsafe { CreateEventW(null(), 1, 0, null()) };
    if handle.is_null() {
        Err(DeleteCollectorError::new(
            DeleteCollectorErrorCode::CreateEvent,
        ))
    } else {
        Ok(OwnedKernelHandle(handle))
    }
}

/// 独占一个 Win32 kernel handle，并允许在线程间安全转移或共享事件调用。
struct OwnedKernelHandle(HANDLE);

// SAFETY: 所有权只由结构本身持有；Win32 kernel handle 可跨线程传递。
unsafe impl Send for OwnedKernelHandle {}
// SAFETY: 共享实例只暴露接受并发调用的 SetEvent/Wait 原始 handle，不改变 Rust 内存。
unsafe impl Sync for OwnedKernelHandle {}

impl OwnedKernelHandle {
    /// 返回借用的原始 handle；调用方不得关闭或保存超过 owner 生命周期。
    const fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedKernelHandle {
    /// 在所有 pending operation 已排空后关闭一次 handle。
    fn drop(&mut self) {
        // SAFETY: 结构只由成功创建的非空/非 INVALID handle 构造并拥有唯一 CloseHandle 权限。
        unsafe { CloseHandle(self.0) };
    }
}

/// 把 Windows 字符串转换为 NUL 结尾 UTF-16。
fn wide_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    //! collector 协议测试不执行删除；深模块集成只删除严格验证的 UUID 根内空目标子目录。

    #[cfg(feature = "delete-validation")]
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::Read;
    use std::ops::Deref;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::thread;
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FILE_GENERIC_READ, FILE_GENERIC_WRITE, OPEN_EXISTING,
    };

    use super::{
        prepare_delete_collector, prepare_delete_collector_with_hooks, wide_null,
        CollectorTestHooks, DeleteCollectorErrorCode, DeleteTargetFact, TestPause,
        COLLECTOR_PROTOCOL_VERSION,
    };
    #[cfg(feature = "delete-validation")]
    use crate::execution::command::DELETE_FOLDERS_ID;
    #[cfg(feature = "delete-validation")]
    use crate::execution::parameter::ParameterValue;
    #[cfg(feature = "delete-validation")]
    use crate::execution::planner::{ExecutionPlanner, PreviewCommandRequest, VerifyRunRequest};
    use crate::execution::safety::{inspect_delete_targets, PathFingerprint, ProtectedPathSet};

    /// 断言失败时也只清理当前测试创建的 UUID 根。
    struct IsolatedRoot(PathBuf);

    impl IsolatedRoot {
        /// 创建不会命中任何既有业务目录的 UUID 测试根。
        fn new(label: &str) -> Self {
            Self(
                std::env::temp_dir()
                    .join("CmdBox")
                    .join(format!("delete-collector-{label}-{}", uuid::Uuid::new_v4())),
            )
        }

        /// 创建带 UUID marker 的真实 Executor 集成测试根。
        fn executor_run() -> Self {
            Self::marked("executor-run")
        }

        /// 创建与真实删除目标根相互独立的 Junction sentinel 根。
        fn executor_sentinel() -> Self {
            Self::marked("executor-sentinel")
        }

        /// 创建带受控前缀、完整 UUID 名称和匹配 marker 的测试根。
        fn marked(prefix: &str) -> Self {
            let identifier = uuid::Uuid::new_v4();
            let root = std::env::temp_dir()
                .join("CmdBox")
                .join(format!("{prefix}-{identifier}"));
            fs::create_dir_all(&root).expect("应创建 Executor UUID 隔离根");
            fs::write(root.join(".cmdbox-executor-test"), identifier.to_string())
                .expect("应写入 Executor 测试 marker");
            Self(root)
        }
    }

    impl Deref for IsolatedRoot {
        type Target = Path;

        /// 允许夹具直接参与 Path API。
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl Drop for IsolatedRoot {
        /// 仅清理当前测试独占的 UUID 根。
        fn drop(&mut self) {
            let expected_parent = std::env::temp_dir().join("CmdBox");
            assert_eq!(
                self.0.parent(),
                Some(expected_parent.as_path()),
                "测试清理目标必须直属 %TEMP%\\CmdBox"
            );
            let name = self
                .0
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .expect("测试清理目标必须有 UTF-8 目录名");
            let valid_prefix = name.starts_with("delete-collector-")
                || name.starts_with("executor-run-")
                || name.starts_with("executor-sentinel-");
            assert!(
                valid_prefix && name.len() >= 36,
                "测试清理目标必须带受控前缀和 UUID"
            );
            let uuid_start = name.len() - 36;
            let identifier = uuid::Uuid::parse_str(&name[uuid_start..])
                .expect("测试清理目标末尾必须是完整 UUID");
            if (name.starts_with("executor-run-") || name.starts_with("executor-sentinel-"))
                && self.0.exists()
            {
                assert_eq!(
                    fs::read_to_string(self.0.join(".cmdbox-executor-test"))
                        .expect("受控 Executor 根清理前 marker 必须存在"),
                    identifier.to_string(),
                    "受控 Executor 根 marker 必须匹配目录 UUID"
                );
            }
            if self.0.exists() {
                fs::remove_dir_all(&self.0).expect("应只清理已验证的 UUID 隔离根");
            }
        }
    }

    /// 取得通过真实 Windows 根级检查的隔离目标指纹。
    fn fingerprint(target: &Path) -> PathFingerprint {
        let protected = ProtectedPathSet::explicit(Vec::new(), Vec::new(), Vec::new());
        inspect_delete_targets(
            &[target.to_str().expect("UUID 测试路径应为 UTF-8").to_owned()],
            &protected,
        )
        .expect("UUID 隔离目录应通过根级安全检查")
        .targets
        .remove(0)
        .fingerprint
    }

    /// 在真实删除测试 Resume 前验证根目录 marker、UUID 和唯一允许删除的直接子目录。
    fn validate_executor_delete_target(root: &Path, target: &Path) {
        let expected_parent = std::env::temp_dir().join("CmdBox");
        assert_eq!(
            root.parent(),
            Some(expected_parent.as_path()),
            "Executor 测试根必须直属 %TEMP%\\CmdBox"
        );
        let root_name = root
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .expect("Executor 测试根必须是 UTF-8");
        let identifier = root_name
            .strip_prefix("executor-run-")
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .expect("Executor 测试根必须带完整 UUID marker");
        assert_eq!(
            fs::read_to_string(root.join(".cmdbox-executor-test"))
                .expect("Executor 测试 marker 必须存在"),
            identifier.to_string(),
            "marker 内容必须与根 UUID 一致"
        );
        assert_eq!(target.parent(), Some(root), "只允许删除隔离根的直接子目录");
        assert_eq!(
            target.file_name().and_then(std::ffi::OsStr::to_str),
            Some("target"),
            "真实删除目标必须使用固定 target 名"
        );
        assert!(target.is_dir(), "Resume 前真实删除目标必须仍为目录");
    }

    /// 验证独立 sentinel 根的前缀、UUID、marker 和保留文件均属于当前测试。
    fn validate_executor_sentinel_root(root: &Path, sentinel: &Path) {
        let expected_parent = std::env::temp_dir().join("CmdBox");
        assert_eq!(root.parent(), Some(expected_parent.as_path()));
        let root_name = root
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .expect("sentinel 测试根必须是 UTF-8");
        let identifier = root_name
            .strip_prefix("executor-sentinel-")
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .expect("sentinel 测试根必须带完整 UUID");
        assert_eq!(
            fs::read_to_string(root.join(".cmdbox-executor-test"))
                .expect("sentinel marker 必须存在"),
            identifier.to_string()
        );
        assert_eq!(sentinel.parent(), Some(root));
        assert!(sentinel.join("keep.txt").is_file());
    }

    /// 保证测试失败时先移除内部 Junction 本身，再由隔离根执行递归清理。
    struct JunctionGuard(PathBuf);

    impl Drop for JunctionGuard {
        /// 只移除当前测试创建的 Junction 根对象，不触碰其目标目录。
        fn drop(&mut self) {
            if fs::symlink_metadata(&self.0).is_ok() {
                fs::remove_dir(&self.0).expect("应先移除仍存在的测试 Junction 根对象");
            }
        }
    }

    /// 从删除目标内部创建指向第二个独立受控 UUID 根的目录 Junction。
    fn create_junction(link: &Path, destination: &Path) -> JunctionGuard {
        let status = Command::new("cmd.exe")
            .args([
                "/D",
                "/C",
                "mklink",
                "/J",
                link.to_str().expect("Junction 测试路径应为 UTF-8"),
                destination.to_str().expect("sentinel 测试路径应为 UTF-8"),
            ])
            .status()
            .expect("应启动固定 mklink /J 命令");
        assert!(status.success(), "UUID 隔离目录 Junction 应创建成功");
        JunctionGuard(link.to_path_buf())
    }

    /// 通过正式 Built-in Definition、Preview、Run 复验取得唯一 Delete Executor 计划。
    #[cfg(feature = "delete-validation")]
    fn verified_delete_plan(target: &Path) -> super::DeleteExecutionPlan {
        let values = BTreeMap::from([(
            "folders".to_owned(),
            ParameterValue::Array(vec![ParameterValue::Text(
                target
                    .to_str()
                    .expect("Executor UUID 测试路径应为 UTF-8")
                    .to_owned(),
            )]),
        )]);
        let planner = ExecutionPlanner::new();
        let preview = planner
            .preview(&PreviewCommandRequest {
                command_block_id: DELETE_FOLDERS_ID.to_owned(),
                expected_revision: 1,
                parameter_values: values.clone(),
            })
            .expect("正式永久删除 Definition 应生成 Preview");
        let verified = planner
            .verify_run(&VerifyRunRequest {
                command_block_id: DELETE_FOLDERS_ID.to_owned(),
                expected_revision: 1,
                parameter_values: values,
                execution_spec_hash: preview.execution_spec_hash,
                safety_confirmation: None,
                target_identity_hash: preview.target_identity_hash,
            })
            .expect("未变化目标应通过正式 Run 全量复验");
        verified
            .into_delete_execution_plan()
            .expect("永久删除授权必须整体转换为 DeleteExecutionPlan")
    }

    /// 并行 drain 受管 stdout/stderr，防止等待进程时形成输出背压。
    struct OutputDrain {
        /// stdout Reader 线程。
        stdout: thread::JoinHandle<Vec<u8>>,
        /// stderr Reader 线程。
        stderr: thread::JoinHandle<Vec<u8>>,
    }

    impl OutputDrain {
        /// 接管一次性输出 Pipe 并启动两个阻塞 Reader。
        fn start(output: crate::process::windows::managed_process::CapturedOutput) -> Self {
            let (mut stdout, mut stderr) = output.into_readers();
            Self {
                stdout: thread::spawn(move || {
                    let mut bytes = Vec::new();
                    stdout.read_to_end(&mut bytes).expect("应 drain stdout");
                    bytes
                }),
                stderr: thread::spawn(move || {
                    let mut bytes = Vec::new();
                    stderr.read_to_end(&mut bytes).expect("应 drain stderr");
                    bytes
                }),
            }
        }

        /// 等待两个 Reader 收到 EOF，并返回原始测试输出。
        fn finish(self) -> (Vec<u8>, Vec<u8>) {
            (
                self.stdout.join().expect("stdout Reader 不应 panic"),
                self.stderr.join().expect("stderr Reader 不应 panic"),
            )
        }
    }

    /// 只尝试一次连接，用于确认 collector lease 结束后 Pipe endpoint 已不存在。
    fn pipe_is_connectable(pipe_leaf: &str) -> bool {
        let full_name = wide_null(std::ffi::OsStr::new(&format!(r"\\.\pipe\{pipe_leaf}")));
        // SAFETY: 名称是 NUL 结尾 UTF-16；成功句柄在返回前立即关闭。
        let handle = unsafe {
            CreateFileW(
                full_name.as_ptr(),
                FILE_GENERIC_READ | FILE_GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            false
        } else {
            // SAFETY: handle 由本次成功 CreateFileW 唯一创建。
            unsafe { CloseHandle(handle) };
            true
        }
    }

    /// 测试客户端独占的同步 Pipe handle。
    struct ClientHandle(HANDLE);

    impl Drop for ClientHandle {
        /// 关闭当前测试创建的客户端连接。
        fn drop(&mut self) {
            // SAFETY: handle 只由成功的 CreateFileW 构造并关闭一次。
            unsafe { CloseHandle(self.0) };
        }
    }

    /// 连接已创建的本地测试 Pipe。
    fn connect_client(pipe_leaf: &str) -> ClientHandle {
        let full_name = wide_null(std::ffi::OsStr::new(&format!(r"\\.\pipe\{pipe_leaf}")));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            // SAFETY: 名称是 NUL 结尾 UTF-16，其他参数符合客户端打开 Named Pipe 的契约。
            let handle = unsafe {
                CreateFileW(
                    full_name.as_ptr(),
                    FILE_GENERIC_READ | FILE_GENERIC_WRITE,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if handle != INVALID_HANDLE_VALUE {
                return ClientHandle(handle);
            }
            assert!(Instant::now() < deadline, "测试客户端应在 deadline 前连接");
            thread::yield_now();
        }
    }

    /// 用同步测试客户端写出一条短协议行。
    fn client_write_line(client: &ClientHandle, line: &str) {
        let bytes = format!("{line}\n").into_bytes();
        client_write_bytes(client, &bytes);
    }

    /// 用同步测试客户端完整写出任意协议字节，用于编码与大小负向矩阵。
    fn client_write_bytes(client: &ClientHandle, bytes: &[u8]) {
        let mut offset = 0_usize;
        while offset < bytes.len() {
            let mut written = 0_u32;
            // SAFETY: 剩余 bytes 和 written 在同步 WriteFile 返回前有效。
            assert_ne!(
                unsafe {
                    WriteFile(
                        client.0,
                        bytes[offset..].as_ptr(),
                        (bytes.len() - offset) as u32,
                        &mut written,
                        std::ptr::null_mut(),
                    )
                },
                0,
                "测试协议字节应写入"
            );
            assert_ne!(written, 0, "同步写入不得零进展");
            offset += written as usize;
        }
    }

    /// 运行一个无删除负向 frame，并统一断言错误、响应、目标与有界完成。
    fn assert_rejected_frame<F>(
        label: &str,
        build_frame: F,
        expected_code: DeleteCollectorErrorCode,
        expected_response: Option<&str>,
    ) where
        F: FnOnce(&super::DeleteCollectorLaunchArgs) -> Vec<u8>,
    {
        let root = IsolatedRoot::new(label);
        let target = root.join("target");
        fs::create_dir_all(&target).expect("应创建 UUID 隔离目标");
        let (args, lease) =
            prepare_delete_collector(vec![fingerprint(&target)], COLLECTOR_PROTOCOL_VERSION)
                .expect("应创建负向协议 collector");
        lease
            .bind_expected_client_pid(std::process::id())
            .expect("应绑定当前测试 PID");
        let frame = build_frame(&args);
        let pipe_leaf = args.pipe_leaf.clone();
        let client = thread::spawn(move || {
            let connection = connect_client(&pipe_leaf);
            client_write_bytes(&connection, &frame);
            client_read_optional_line(&connection)
        });

        let started = Instant::now();
        let error = lease.finish().expect_err("负向协议必须 fail-closed");
        let elapsed = started.elapsed();
        let response = client.join().expect("负向测试客户端不应 panic");

        assert_eq!(error.code, expected_code, "case={label}");
        assert_eq!(response.as_deref(), expected_response, "case={label}");
        assert!(
            response
                .as_deref()
                .is_none_or(|line| !line.starts_with("APPROVE|")),
            "负向 case 不得收到 APPROVE：{label}"
        );
        assert!(elapsed < Duration::from_secs(2), "finish 必须有界：{label}");
        assert!(target.is_dir(), "负向 frame 不得改变目标：{label}");
    }

    /// 用同步测试客户端读取一条短 UTF-8 响应行。
    fn client_read_line(client: &ClientHandle) -> String {
        let mut output = Vec::new();
        loop {
            let mut byte = 0_u8;
            let mut read = 0_u32;
            // SAFETY: byte 和 read 在同步 ReadFile 返回前有效。
            assert_ne!(
                unsafe { ReadFile(client.0, &mut byte, 1, &mut read, std::ptr::null_mut(),) },
                0,
                "测试响应应可读"
            );
            assert_eq!(read, 1);
            if byte == b'\n' {
                return String::from_utf8(output).expect("服务端响应应为 UTF-8");
            }
            output.push(byte);
        }
    }

    /// 读取可选响应；服务端未写出任何字节即关闭时返回 `None`。
    fn client_read_optional_line(client: &ClientHandle) -> Option<String> {
        let mut output = Vec::new();
        loop {
            let mut byte = 0_u8;
            let mut read = 0_u32;
            // SAFETY: byte 和 read 在同步 ReadFile 返回前有效。
            if unsafe { ReadFile(client.0, &mut byte, 1, &mut read, std::ptr::null_mut()) } == 0
                || read == 0
            {
                return if output.is_empty() {
                    None
                } else {
                    Some(String::from_utf8(output).expect("部分服务端响应仍应为 UTF-8"))
                };
            }
            if byte == b'\n' {
                return Some(String::from_utf8(output).expect("服务端响应应为 UTF-8"));
            }
            output.push(byte);
        }
    }

    /// 验证 cancel 先通过共享 gate 线性化后，fresh 目标也不再收到任何 APPROVE 字节。
    #[test]
    fn cancellation_linearized_before_approval_writes_no_approval() {
        let root = IsolatedRoot::new("approval-cancel");
        let target = root.join("target");
        fs::create_dir_all(&target).expect("应创建 UUID 隔离目标");
        let pause = TestPause::new();
        let hooks = CollectorTestHooks {
            before_approval_gate: Some(pause.clone()),
            pending_read: None,
        };
        let (args, lease) = prepare_delete_collector_with_hooks(
            vec![fingerprint(&target)],
            COLLECTOR_PROTOCOL_VERSION,
            hooks,
        )
        .expect("应创建带授权暂停点的 collector");
        lease
            .bind_expected_client_pid(std::process::id())
            .expect("应绑定当前测试 PID");

        let client = thread::spawn(move || {
            let connection = connect_client(&args.pipe_leaf);
            client_write_line(
                &connection,
                &format!("BEGIN|{}|{}|0", args.token, args.generation),
            );
            client_read_optional_line(&connection)
        });
        pause.wait_until_reached();
        lease.cancellation().cancel();
        pause.release_watcher();
        let error = lease.finish().expect_err("cancel 后 watcher 必须终止");
        let response = client.join().expect("测试客户端不应 panic");

        assert_eq!(error.code, DeleteCollectorErrorCode::Cancelled);
        assert_eq!(
            response, None,
            "cancel 先线性化时不得写出部分或完整 APPROVE"
        );
        assert!(target.is_dir(), "未授权目标必须保持存在");
    }

    /// 验证客户端已连接但握手 ReadFile pending 时，cancel 可确定性解除等待。
    #[test]
    fn cancellation_unblocks_pending_handshake_read() {
        let root = IsolatedRoot::new("pending-read-cancel");
        let target = root.join("target");
        fs::create_dir_all(&target).expect("应创建 UUID 隔离目标");
        let pause = TestPause::new();
        let hooks = CollectorTestHooks {
            before_approval_gate: None,
            pending_read: Some(pause.clone()),
        };
        let (args, lease) = prepare_delete_collector_with_hooks(
            vec![fingerprint(&target)],
            COLLECTOR_PROTOCOL_VERSION,
            hooks,
        )
        .expect("应创建带 pending read 暂停点的 collector");
        lease
            .bind_expected_client_pid(std::process::id())
            .expect("应绑定当前测试 PID");

        let client = thread::spawn(move || {
            let connection = connect_client(&args.pipe_leaf);
            client_read_optional_line(&connection)
        });
        pause.wait_until_reached();
        lease.cancellation().cancel();
        pause.release_watcher();
        let error = lease.finish().expect_err("pending handshake 应被取消");
        let response = client.join().expect("测试客户端不应 panic");

        assert_eq!(error.code, DeleteCollectorErrorCode::Cancelled);
        assert_eq!(response, None);
        assert!(target.is_dir(), "握手未完成不得产生删除副作用");
    }

    /// 验证可解码但不符合 BEGIN 契约的帧都稳定拒绝且不授权。
    #[test]
    fn decoded_malformed_begin_matrix_is_denied_without_approval() {
        for (label, kind) in [
            ("wrong-token", 0_u8),
            ("wrong-generation", 1_u8),
            ("wrong-index", 2_u8),
            ("missing-field", 3_u8),
            ("extra-field", 4_u8),
        ] {
            assert_rejected_frame(
                label,
                move |args| {
                    match kind {
                        0 => format!("BEGIN|wrong-token|{}|0\n", args.generation),
                        1 => format!("BEGIN|{}|wrong-generation|0\n", args.token),
                        2 => format!("BEGIN|{}|{}|1\n", args.token, args.generation),
                        3 => format!("BEGIN|{}|{}\n", args.token, args.generation),
                        4 => format!("BEGIN|{}|{}|0|extra\n", args.token, args.generation),
                        _ => unreachable!("表驱动用例类型必须已列举"),
                    }
                    .into_bytes()
                },
                DeleteCollectorErrorCode::InvalidProtocol,
                Some("DENY|invalidProtocol"),
            );
        }
    }

    /// 验证超过 512 字节的单行在协议解码前被拒绝。
    #[test]
    fn oversized_line_is_rejected_without_approval() {
        assert_rejected_frame(
            "oversized-line",
            |_| {
                let mut frame = vec![b'A'; super::MAX_LINE_BYTES + 1];
                frame.push(b'\n');
                frame
            },
            DeleteCollectorErrorCode::InputTooLarge,
            None,
        );
    }

    /// 验证非 UTF-8 字节在协议解码前被拒绝。
    #[test]
    fn invalid_utf8_is_rejected_without_approval() {
        assert_rejected_frame(
            "invalid-utf8",
            |_| vec![0xff, b'\n'],
            DeleteCollectorErrorCode::InvalidEncoding,
            None,
        );
    }

    /// 验证内嵌 NUL 的 UTF-8 行在协议解码前被拒绝。
    #[test]
    fn embedded_nul_is_rejected_without_approval() {
        assert_rejected_frame(
            "embedded-nul",
            |_| vec![b'B', 0, b'\n'],
            DeleteCollectorErrorCode::InvalidEncoding,
            None,
        );
    }

    /// 验证合法 BEGIN 与未消费尾随帧同批到达时不会先授权。
    #[test]
    fn trailing_unconsumed_message_is_rejected_without_approval() {
        assert_rejected_frame(
            "trailing-message",
            |args| {
                format!(
                    "BEGIN|{}|{}|0\nTRAILING|{}|{}|0\n",
                    args.token, args.generation, args.token, args.generation
                )
                .into_bytes()
            },
            DeleteCollectorErrorCode::InvalidProtocol,
            Some("DENY|invalidProtocol"),
        );
    }

    /// 验证只有 PID、token、generation 和 fresh fingerprint 全通过后才授权并接收事实。
    #[test]
    fn approves_fresh_target_and_collects_exactly_one_fact_without_deleting() {
        let root = IsolatedRoot::new("success");
        let target = root.join("target");
        fs::create_dir_all(&target).expect("应创建 UUID 隔离目标");
        let (args, lease) =
            prepare_delete_collector(vec![fingerprint(&target)], COLLECTOR_PROTOCOL_VERSION)
                .expect("应创建 collector");
        lease
            .bind_expected_client_pid(std::process::id())
            .expect("应在客户端连接前绑定当前测试 PID");

        let client_args = args.clone();
        let target_for_client = target.clone();
        let preserved = root.join("preserved-after-success");
        let preserved_for_client = preserved.clone();
        let client = thread::spawn(move || {
            let client = connect_client(&client_args.pipe_leaf);
            client_write_line(
                &client,
                &format!("BEGIN|{}|{}|0", client_args.token, client_args.generation),
            );
            assert_eq!(
                client_read_line(&client),
                format!("APPROVE|{}|{}|0", client_args.token, client_args.generation)
            );
            fs::rename(target_for_client, preserved_for_client)
                .expect("用重命名模拟目标根消失，不执行删除");
            client_write_line(
                &client,
                &format!("SUCCESS|{}|{}|0", client_args.token, client_args.generation),
            );
            assert_eq!(
                client_read_line(&client),
                format!("ACK|{}|{}|0", client_args.token, client_args.generation)
            );
        });

        let result = lease.finish().expect("完整协议应产生可信事实");
        client.join().expect("测试客户端不应 panic");
        assert_eq!(
            result.target_facts,
            vec![DeleteTargetFact::Success { target_index: 0 }]
        );
        assert!(!target.exists(), "SUCCESS 后原目标路径应确实不存在");
        assert!(preserved.is_dir(), "测试只重命名并保留原目录，不执行删除");
    }

    /// 验证固定脚本报告受限 FAILURE 时保留目标并得到逐目标 ACK。
    #[test]
    fn records_fixed_failure_fact_without_changing_target() {
        let root = IsolatedRoot::new("failure");
        let target = root.join("target");
        fs::create_dir_all(&target).expect("应创建 UUID 隔离目标");
        let (args, lease) =
            prepare_delete_collector(vec![fingerprint(&target)], COLLECTOR_PROTOCOL_VERSION)
                .expect("应创建 collector");
        lease
            .bind_expected_client_pid(std::process::id())
            .expect("应绑定当前测试 PID");

        let client_args = args.clone();
        let client = thread::spawn(move || {
            let connection = connect_client(&client_args.pipe_leaf);
            client_write_line(
                &connection,
                &format!("BEGIN|{}|{}|0", client_args.token, client_args.generation),
            );
            assert_eq!(
                client_read_line(&connection),
                format!("APPROVE|{}|{}|0", client_args.token, client_args.generation)
            );
            client_write_line(
                &connection,
                &format!(
                    "FAILURE|{}|{}|0|removeFailed",
                    client_args.token, client_args.generation
                ),
            );
            assert_eq!(
                client_read_line(&connection),
                format!("ACK|{}|{}|0", client_args.token, client_args.generation)
            );
        });

        let result = lease.finish().expect("固定 FAILURE 应形成可信失败事实");
        client.join().expect("测试客户端不应 panic");
        assert_eq!(
            result.target_facts,
            vec![DeleteTargetFact::Failure {
                target_index: 0,
                error_code: "removeFailed".to_owned(),
            }]
        );
        assert!(target.is_dir(), "collector 不得因 FAILURE 改变目标");
    }

    /// 验证客户端虚报 SUCCESS 但目标仍存在时不会获得 ACK 或成功事实。
    #[test]
    fn rejects_claimed_success_when_target_root_still_exists() {
        let root = IsolatedRoot::new("false-success");
        let target = root.join("target");
        fs::create_dir_all(&target).expect("应创建 UUID 隔离目标");
        let (args, lease) =
            prepare_delete_collector(vec![fingerprint(&target)], COLLECTOR_PROTOCOL_VERSION)
                .expect("应创建 collector");
        lease
            .bind_expected_client_pid(std::process::id())
            .expect("应绑定当前测试 PID");

        let client = thread::spawn(move || {
            let connection = connect_client(&args.pipe_leaf);
            client_write_line(
                &connection,
                &format!("BEGIN|{}|{}|0", args.token, args.generation),
            );
            assert_eq!(
                client_read_line(&connection),
                format!("APPROVE|{}|{}|0", args.token, args.generation)
            );
            client_write_line(
                &connection,
                &format!("SUCCESS|{}|{}|0", args.token, args.generation),
            );
        });

        let error = lease.finish().expect_err("目标仍存在时不能接受 SUCCESS");
        client.join().expect("测试客户端不应 panic");
        assert_eq!(error.code, DeleteCollectorErrorCode::InvalidProtocol);
        assert!(target.is_dir(), "伪 SUCCESS 不得改变目标");
    }

    /// 验证同路径对象在 BEGIN 前被替换后只能收到 DENY。
    #[test]
    fn denies_when_target_identity_changes_before_begin() {
        let root = IsolatedRoot::new("changed");
        let target = root.join("target");
        let original = root.join("original");
        fs::create_dir_all(&target).expect("应创建 UUID 隔离目标");
        let expected = fingerprint(&target);
        let (args, lease) = prepare_delete_collector(vec![expected], COLLECTOR_PROTOCOL_VERSION)
            .expect("应创建 collector");
        fs::rename(&target, &original).expect("应在隔离根内保留旧对象并让原路径空出");
        fs::create_dir(&target).expect("应在相同路径创建不同身份的新目录");
        lease
            .bind_expected_client_pid(std::process::id())
            .expect("应绑定当前测试 PID");

        let client = thread::spawn(move || {
            let connection = connect_client(&args.pipe_leaf);
            client_write_line(
                &connection,
                &format!("BEGIN|{}|{}|0", args.token, args.generation),
            );
            assert_eq!(client_read_line(&connection), "DENY|safetyChanged");
        });
        let error = lease.finish().expect_err("身份漂移必须 fail-closed");
        client.join().expect("测试客户端不应 panic");
        assert_eq!(error.code, DeleteCollectorErrorCode::SafetyChanged);
        assert!(target.is_dir());
        assert!(original.is_dir());
    }

    /// 验证客户端 PID 不匹配时在读取 BEGIN 前就拒绝连接。
    #[test]
    fn rejects_client_process_that_is_not_bound_session_pid() {
        let root = IsolatedRoot::new("pid");
        let target = root.join("target");
        fs::create_dir_all(&target).expect("应创建 UUID 隔离目标");
        let (args, lease) =
            prepare_delete_collector(vec![fingerprint(&target)], COLLECTOR_PROTOCOL_VERSION)
                .expect("应创建 collector");
        lease
            .bind_expected_client_pid(std::process::id().wrapping_add(1).max(1))
            .expect("应绑定模拟的不同 PID");
        let client = thread::spawn(move || {
            let _connection = connect_client(&args.pipe_leaf);
            // 保持客户端存活到 watcher 完成 PID 查询；立即 Close 会让 Windows 合法返回
            // ERROR_PIPE_NOT_CONNECTED，从而只能证明断连而不能稳定证明 PID mismatch 分支。
            thread::sleep(Duration::from_millis(500));
        });
        let error = lease.finish().expect_err("错误 PID 必须 fail-closed");
        client.join().expect("测试客户端不应 panic");
        assert_eq!(error.code, DeleteCollectorErrorCode::ClientPidMismatch);
        assert!(target.is_dir());
    }

    /// 验证 Drop/Cancel 能解除尚未连接的 Overlapped Connect 并及时 join。
    #[test]
    fn cancellation_unblocks_pending_connect_without_client() {
        let root = IsolatedRoot::new("cancel");
        let target = root.join("target");
        fs::create_dir_all(&target).expect("应创建 UUID 隔离目标");
        let (_args, lease) =
            prepare_delete_collector(vec![fingerprint(&target)], COLLECTOR_PROTOCOL_VERSION)
                .expect("应创建 collector");
        let started = Instant::now();
        lease.cancel();
        let error = lease.finish().expect_err("取消应终止 watcher");
        assert_eq!(error.code, DeleteCollectorErrorCode::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(target.is_dir());
    }

    /// 验证 index 0 失败并 ACK 后不会阻断 index 1 的独立授权与成功事实。
    #[test]
    fn continues_multi_target_protocol_after_prior_failure() {
        let root = IsolatedRoot::new("multi-target");
        let first = root.join("first");
        let second = root.join("second");
        let preserved_second = root.join("preserved-second");
        fs::create_dir_all(&first).expect("应创建第一个 UUID 隔离目标");
        fs::create_dir_all(&second).expect("应创建第二个 UUID 隔离目标");
        let (args, lease) = prepare_delete_collector(
            vec![fingerprint(&first), fingerprint(&second)],
            COLLECTOR_PROTOCOL_VERSION,
        )
        .expect("应创建双目标 collector");
        lease
            .bind_expected_client_pid(std::process::id())
            .expect("应绑定当前测试 PID");

        let client_args = args.clone();
        let second_for_client = second.clone();
        let preserved_for_client = preserved_second.clone();
        let client = thread::spawn(move || {
            let connection = connect_client(&client_args.pipe_leaf);
            client_write_line(
                &connection,
                &format!("BEGIN|{}|{}|0", client_args.token, client_args.generation),
            );
            assert_eq!(
                client_read_line(&connection),
                format!("APPROVE|{}|{}|0", client_args.token, client_args.generation)
            );
            client_write_line(
                &connection,
                &format!(
                    "FAILURE|{}|{}|0|removeFailed",
                    client_args.token, client_args.generation
                ),
            );
            assert_eq!(
                client_read_line(&connection),
                format!("ACK|{}|{}|0", client_args.token, client_args.generation)
            );

            client_write_line(
                &connection,
                &format!("BEGIN|{}|{}|1", client_args.token, client_args.generation),
            );
            assert_eq!(
                client_read_line(&connection),
                format!("APPROVE|{}|{}|1", client_args.token, client_args.generation)
            );
            fs::rename(second_for_client, preserved_for_client)
                .expect("用重命名模拟第二目标成功，不执行删除");
            client_write_line(
                &connection,
                &format!("SUCCESS|{}|{}|1", client_args.token, client_args.generation),
            );
            assert_eq!(
                client_read_line(&connection),
                format!("ACK|{}|{}|1", client_args.token, client_args.generation)
            );
        });

        let result = lease.finish().expect("双目标协议应自然完成");
        client.join().expect("测试客户端不应 panic");
        assert_eq!(
            result.target_facts,
            vec![
                DeleteTargetFact::Failure {
                    target_index: 0,
                    error_code: "removeFailed".to_owned(),
                },
                DeleteTargetFact::Success { target_index: 1 },
            ]
        );
        assert!(first.is_dir(), "失败目标应保持存在");
        assert!(preserved_second.is_dir(), "第二目标只被安全重命名并保留");
    }

    /// 真实运行固定 PowerShell，仅删除 UUID 根内 exact target，并证明内部 Junction 不影响 sentinel。
    #[cfg(feature = "delete-validation")]
    #[test]
    fn execution_plan_deletes_only_validated_target_and_preserves_junction_sentinel() {
        let root = IsolatedRoot::executor_run();
        let sentinel_root = IsolatedRoot::executor_sentinel();
        let target = root.join("target");
        let sentinel = sentinel_root.join("sentinel");
        let internal_junction = target.join("outside-sentinel");
        fs::create_dir_all(&target).expect("应创建真实删除目标");
        fs::create_dir_all(&sentinel).expect("应创建目标外 sentinel");
        fs::write(sentinel.join("keep.txt"), b"keep").expect("应创建 sentinel 内容");
        let junction_guard = create_junction(&internal_junction, &sentinel);

        let mut prepared = verified_delete_plan(&target)
            .prepare()
            .expect("深模块应完成挂起准备与 PID 绑定");
        assert_ne!(prepared.process_id(), 0);
        let artifact_directory = prepared.temporary_directory().to_path_buf();
        let transport_pipe = prepared.transport_pipe_leaf().to_owned();
        let _cancellation = prepared.cancellation().clone();
        let output = OutputDrain::start(prepared.take_output().expect("应取得受管输出"));
        validate_executor_delete_target(&root, &target);
        validate_executor_sentinel_root(&sentinel_root, &sentinel);

        let running = prepared.resume().expect("安全目标应允许 Resume");
        let (process, collector, policy) = running.into_parts();
        let exit_code = process.wait().expect("应等待固定脚本退出");
        process.wait_job_empty().expect("真实删除 Job 应为空");
        let result = collector.finish().expect("应取得真实 SUCCESS 事实");
        assert_eq!(policy.version(), 1);
        drop(process);
        let (_stdout, stderr) = output.finish();
        drop(junction_guard);

        assert_eq!(
            exit_code,
            0,
            "固定脚本应成功，stderr={}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(
            result.target_facts,
            vec![DeleteTargetFact::Success { target_index: 0 }]
        );
        assert!(!target.exists(), "唯一授权的 target 应被永久删除");
        assert!(
            sentinel.join("keep.txt").is_file(),
            "Junction 外 sentinel 必须完整"
        );
        assert!(
            !artifact_directory.exists(),
            "受管 Artifact 必须由 RAII 清理"
        );
        assert!(
            !pipe_is_connectable(&transport_pipe),
            "collector Pipe 不得残留"
        );
    }

    /// 验证计划准备后、Resume 前目标身份漂移时，固定脚本只能收到 DENY 且不删除任一对象。
    #[cfg(feature = "delete-validation")]
    #[test]
    fn execution_plan_denies_identity_drift_before_resume_without_deleting() {
        let root = IsolatedRoot::executor_run();
        let target = root.join("target");
        let original = root.join("original");
        let sentinel = root.join("sentinel.txt");
        fs::create_dir_all(&target).expect("应创建原始目标");
        fs::write(&sentinel, b"keep").expect("应创建非目标 sentinel");

        let mut prepared = verified_delete_plan(&target)
            .prepare()
            .expect("深模块应完成挂起准备与 PID 绑定");
        let artifact_directory = prepared.temporary_directory().to_path_buf();
        let transport_pipe = prepared.transport_pipe_leaf().to_owned();
        let output = OutputDrain::start(prepared.take_output().expect("应取得受管输出"));
        fs::rename(&target, &original).expect("应保留原对象并让目标路径空出");
        fs::create_dir(&target).expect("应在 Resume 前创建不同身份的新目标");

        let running = prepared
            .resume()
            .expect("Resume 只启动固定客户端，不代表授权删除");
        let (process, collector, _policy) = running.into_parts();
        let exit_code = process.wait().expect("应等待 DENY 后退出");
        process.wait_job_empty().expect("DENY 后 Job 应为空");
        let collector_error = collector.finish().expect_err("身份漂移必须 fail-closed");
        drop(process);
        let (_stdout, _stderr) = output.finish();

        assert_eq!(exit_code, 70, "固定脚本收到 DENY 后应使用契约退出码");
        assert_eq!(
            collector_error.code,
            DeleteCollectorErrorCode::SafetyChanged
        );
        assert!(target.is_dir(), "重建目标不得被删除");
        assert!(original.is_dir(), "原身份对象不得被删除");
        assert!(sentinel.is_file(), "非目标 sentinel 必须存在");
        assert!(!artifact_directory.exists(), "失败路径 Artifact 必须清理");
        assert!(
            !pipe_is_connectable(&transport_pipe),
            "失败路径 Pipe 不得残留"
        );
    }
}
