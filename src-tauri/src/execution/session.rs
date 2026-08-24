//! 已验证 Execution 的 Session 组合与有界事件接收端。
//!
//! Session 在恢复进程前完成事件队列、输出 Reader、Active 索引和取消入口绑定。运行线程只
//! 负责等待根进程、关闭 Job、等待输出 Drain 完成并发布唯一终态，不持有 Manager 全局锁。
//! 唯一生产入口只消费 Planner 生成的 `VerifiedExecution`，不能接受脚本或 Runner 旁路参数。

use std::collections::VecDeque;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::io;
#[cfg(test)]
use std::path::Path;
use std::sync::mpsc::{RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::execution::artifact::ArtifactError;
use crate::execution::manager::{
    lock_unpoisoned, ActiveExecution, ExecutionControlState, ExecutionId, ExecutionManager,
};
use crate::execution::outcome::{Outcome, OutcomePolicy};
use crate::execution::output::{OutputBatch, OutputCapture};
use crate::execution::planner::VerifiedExecution;
use crate::process::windows::managed_process::{
    ManagedProcess, ManagedProcessError, CMDBOX_CANCEL_EXIT_CODE,
};

/// 单个 Session 最多保留的待消费事件数；输出满时丢弃，生命周期事件始终保留。
const SESSION_EVENT_CAPACITY: usize = 64;

/// Execution Session 启动失败。
#[derive(Debug)]
pub enum ExecutionStartError {
    /// destructive Definition 已验证，但可信 Executor 尚未接入当前构建。
    ExecutorUnavailable,
    /// 无法创建已验证脚本的临时 Artifact。
    Artifact(ArtifactError),
    /// 无法准备或恢复受管进程。
    Process(ManagedProcessError),
    /// 无法创建 Session 后台线程。
    Thread(io::Error),
}

impl Display for ExecutionStartError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExecutorUnavailable => formatter.write_str("可信 Execution Executor 尚未就绪"),
            Self::Artifact(source) => write!(formatter, "Execution Artifact 创建失败：{source}"),
            Self::Process(source) => write!(formatter, "Execution 进程启动失败：{source}"),
            Self::Thread(source) => write!(formatter, "Execution 后台线程创建失败：{source}"),
        }
    }
}

impl Error for ExecutionStartError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ExecutorUnavailable => None,
            Self::Artifact(source) => Some(source),
            Self::Process(source) => Some(source),
            Self::Thread(source) => Some(source),
        }
    }
}

/// Session 对调用方发布的生命周期与输出事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionEvent {
    /// Active 已登记、事件端已绑定，进程即将 Resume。
    Started {
        /// 当前 Execution ID。
        execution_id: ExecutionId,
        /// 受管根进程 PID，仅供观测。
        process_id: u32,
    },
    /// 一批按协调器观察顺序排列的 stdout/stderr 文本。
    Output {
        /// 当前 Execution ID。
        execution_id: ExecutionId,
        /// 有界输出 Batch。
        batch: OutputBatch,
    },
    /// 根进程自然结束或在取消到达前已经结束。
    Finished {
        /// 当前 Execution ID。
        execution_id: ExecutionId,
        /// Windows PowerShell 原始 Exit Code。
        exit_code: u32,
        /// Command Block Policy 对原始 Exit Code 的业务解释。
        outcome: Outcome,
        /// 从 Resume 到终态发布前的后端耗时。
        duration: Duration,
        /// Session 事件消费者过慢造成且尚未随 Output 报告的字节数。
        dropped_output_bytes: u64,
    },
    /// 已接受取消且确认 Job 根进程结束。
    Cancelled {
        /// 当前 Execution ID。
        execution_id: ExecutionId,
        /// 取消没有自然完成的业务事实，固定为 `none`。
        outcome: Outcome,
        /// 从 Resume 到终态发布前的后端耗时。
        duration: Duration,
        /// Session 事件消费者过慢造成且尚未随 Output 报告的字节数。
        dropped_output_bytes: u64,
    },
    /// Resume 后等待或输出后台任务发生内部失败。
    Failed {
        /// 当前 Execution ID。
        execution_id: ExecutionId,
        /// 面向开发者的稳定失败说明。
        message: String,
        /// Core 内部失败不能冒充命令业务失败，固定为 `none`。
        outcome: Outcome,
        /// 从 Resume 到终态发布前的后端耗时。
        duration: Duration,
        /// Session 事件消费者过慢造成且尚未随 Output 报告的字节数。
        dropped_output_bytes: u64,
    },
}

impl ExecutionEvent {
    /// 返回当前事件是否为唯一生命周期终态。
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Finished { .. } | Self::Cancelled { .. } | Self::Failed { .. }
        )
    }

    /// 把尚未报告的 Session 丢弃字节并入终态。
    fn add_terminal_dropped_bytes(&mut self, dropped: u64) {
        let target = match self {
            Self::Finished {
                dropped_output_bytes,
                ..
            }
            | Self::Cancelled {
                dropped_output_bytes,
                ..
            }
            | Self::Failed {
                dropped_output_bytes,
                ..
            } => dropped_output_bytes,
            Self::Started { .. } | Self::Output { .. } => return,
        };
        *target = target.saturating_add(dropped);
    }
}

/// 启动成功后原子返回的 Execution ID 与专属事件接收端。
pub struct StartedExecution {
    /// 当前 Execution ID。
    pub execution_id: ExecutionId,
    /// 在进程 Resume 前已经绑定的专属接收端。
    pub events: ExecutionEventReceiver,
    /// 测试用于验证终态前已清理当前 Artifact 的唯一目录。
    #[cfg(test)]
    temporary_directory: std::path::PathBuf,
}

#[cfg(test)]
impl StartedExecution {
    /// 返回当前 Session 自有临时目录，只用于同模块清理断言。
    fn temporary_directory(&self) -> &Path {
        &self.temporary_directory
    }
}

/// 有界事件队列的接收端。
pub struct ExecutionEventReceiver {
    /// 与生产端共享的队列和唤醒条件。
    queue: Arc<EventQueue>,
}

impl ExecutionEventReceiver {
    /// 在超时前等待下一事件；生产端关闭且队列为空时返回 Disconnected。
    pub fn recv_timeout(&self, timeout: Duration) -> Result<ExecutionEvent, RecvTimeoutError> {
        let deadline = Instant::now() + timeout;
        let mut state = lock_unpoisoned(&self.queue.state);
        loop {
            if let Some(event) = state.events.pop_front() {
                return Ok(event);
            }
            if state.closed {
                return Err(RecvTimeoutError::Disconnected);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(RecvTimeoutError::Timeout);
            }
            let wait = deadline.saturating_duration_since(now);
            let (next_state, _) = self
                .queue
                .available
                .wait_timeout(state, wait)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = next_state;
        }
    }

    /// 非阻塞读取下一事件。
    pub fn try_recv(&self) -> Result<ExecutionEvent, TryRecvError> {
        let mut state = lock_unpoisoned(&self.queue.state);
        if let Some(event) = state.events.pop_front() {
            return Ok(event);
        }
        if state.closed {
            Err(TryRecvError::Disconnected)
        } else {
            Err(TryRecvError::Empty)
        }
    }

    /// 返回当前待消费事件数，只用于证明测试中的队列容量边界。
    #[cfg(test)]
    fn pending_len(&self) -> usize {
        lock_unpoisoned(&self.queue.state).events.len()
    }

    /// 在测试期限内等待生产端关闭；等待期间不消费事件，构造确定的慢消费者压力。
    #[cfg(test)]
    fn wait_until_closed(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = lock_unpoisoned(&self.queue.state);
        while !state.closed {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let wait = deadline.saturating_duration_since(now);
            let (next_state, wait_result) = self
                .queue
                .available
                .wait_timeout(state, wait)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = next_state;
            if wait_result.timed_out() && !state.closed {
                return false;
            }
        }
        true
    }
}

/// 有界事件队列及其条件变量。
struct EventQueue {
    /// 受 Mutex 保护的事件、关闭位和延迟丢弃计数。
    state: Mutex<EventQueueState>,
    /// 新事件或关闭状态到达时唤醒接收端。
    available: Condvar,
}

/// 有界事件队列的可变状态。
struct EventQueueState {
    /// 尚未由调用方消费的事件。
    events: VecDeque<ExecutionEvent>,
    /// 后端已发布终态且不会再发送事件。
    closed: bool,
    /// 因队列已满被移除且尚未报告的输出原始字节数。
    deferred_dropped_bytes: u64,
}

/// 可由多个后台线程克隆的事件生产端。
#[derive(Clone)]
struct EventSink {
    /// 与专属接收端共享的有界队列。
    queue: Arc<EventQueue>,
}

impl EventSink {
    /// 创建一组专属生产端和接收端。
    fn channel() -> (Self, ExecutionEventReceiver) {
        let queue = Arc::new(EventQueue {
            state: Mutex::new(EventQueueState {
                events: VecDeque::with_capacity(SESSION_EVENT_CAPACITY),
                closed: false,
                deferred_dropped_bytes: 0,
            }),
            available: Condvar::new(),
        });
        (
            Self {
                queue: Arc::clone(&queue),
            },
            ExecutionEventReceiver { queue },
        )
    }

    /// 推入不会被丢弃的 Started 事件。
    fn push_started(&self, event: ExecutionEvent) {
        let mut state = lock_unpoisoned(&self.queue.state);
        state.events.push_back(event);
        self.queue.available.notify_one();
    }

    /// 非阻塞推入 Output；队列满时累计字节而不阻塞 Pipe Reader 或外部进程。
    fn push_output(&self, execution_id: ExecutionId, mut batch: OutputBatch) {
        let mut state = lock_unpoisoned(&self.queue.state);
        if state.closed {
            return;
        }
        if state.events.len() >= SESSION_EVENT_CAPACITY {
            state.deferred_dropped_bytes = state
                .deferred_dropped_bytes
                .saturating_add(batch_total_bytes(&batch));
            return;
        }
        batch.dropped_bytes_before = batch
            .dropped_bytes_before
            .saturating_add(std::mem::take(&mut state.deferred_dropped_bytes));
        state.events.push_back(ExecutionEvent::Output {
            execution_id,
            batch,
        });
        self.queue.available.notify_one();
    }

    /// 记录 OutputCapture 尚未随 Batch 投递的丢弃字节。
    fn note_dropped_bytes(&self, dropped: u64) {
        let mut state = lock_unpoisoned(&self.queue.state);
        state.deferred_dropped_bytes = state.deferred_dropped_bytes.saturating_add(dropped);
    }

    /// 发布唯一终态；必要时移除最旧 Output 腾出位置，并关闭生产端。
    fn push_terminal(&self, mut event: ExecutionEvent) {
        let mut state = lock_unpoisoned(&self.queue.state);
        if state.closed {
            return;
        }
        while state.events.len() >= SESSION_EVENT_CAPACITY {
            let Some(index) = state
                .events
                .iter()
                .position(|queued| matches!(queued, ExecutionEvent::Output { .. }))
            else {
                break;
            };
            if let Some(ExecutionEvent::Output { batch, .. }) = state.events.remove(index) {
                state.deferred_dropped_bytes = state
                    .deferred_dropped_bytes
                    .saturating_add(batch_total_bytes(&batch));
            }
        }
        event.add_terminal_dropped_bytes(std::mem::take(&mut state.deferred_dropped_bytes));
        state.events.push_back(event);
        state.closed = true;
        self.queue.available.notify_all();
    }

    /// 启动失败且接收端不会返回给调用方时关闭队列。
    fn close(&self) {
        let mut state = lock_unpoisoned(&self.queue.state);
        state.closed = true;
        self.queue.available.notify_all();
    }
}

impl ExecutionManager {
    /// 启动一个已由 Planner 全量复验的 Execution，并返回专属有界事件接收端。
    ///
    /// 本方法消费授权值后才创建 Artifact；调用方不能传入脚本、可执行文件、Runner 参数或
    /// 工作目录。进程恢复前会完成 Output、Active 与取消能力绑定。
    pub fn start(
        &self,
        verified: VerifiedExecution,
    ) -> Result<StartedExecution, ExecutionStartError> {
        if !verified.launch_ready() {
            return Err(ExecutionStartError::ExecutorUnavailable);
        }
        let (launch, outcome_policy) = verified
            .into_session_parts()
            .map_err(ExecutionStartError::Artifact)?;
        #[cfg(test)]
        let temporary_directory = launch.temporary_directory().to_path_buf();
        let execution_id = ExecutionId::new_v4();
        let mut prepared = ManagedProcess::prepare(launch).map_err(ExecutionStartError::Process)?;
        let process_id = prepared.process_id();
        let output = prepared
            .take_output()
            .expect("PreparedManagedProcess 必须拥有一次输出读端");
        let output_capture = OutputCapture::start(output);
        let (event_sink, events) = EventSink::channel();
        let active_state = Arc::new(Mutex::new(ExecutionControlState::Running));
        let active = Arc::new(ActiveExecution {
            state: Arc::clone(&active_state),
            cancellation: prepared.cancellation(),
        });
        self.insert(execution_id, active);
        event_sink.push_started(ExecutionEvent::Started {
            execution_id,
            process_id,
        });

        let output_sink = event_sink.clone();
        let output_worker = match thread::Builder::new()
            .name(format!("cmdbox-output-forward-{execution_id}"))
            .spawn(move || forward_output(execution_id, output_capture, output_sink))
        {
            Ok(worker) => worker,
            Err(error) => {
                self.remove(execution_id);
                event_sink.close();
                return Err(ExecutionStartError::Thread(error));
            }
        };

        let process = match prepared.resume() {
            Ok(process) => process,
            Err(error) => {
                self.remove(execution_id);
                let _ = output_worker.join();
                event_sink.close();
                return Err(ExecutionStartError::Process(error));
            }
        };
        let resumed_at = Instant::now();
        let supervisor_manager = self.clone();
        let supervisor_sink = event_sink.clone();
        let supervisor = thread::Builder::new()
            .name(format!("cmdbox-execution-{execution_id}"))
            .spawn(move || {
                supervise_execution(
                    process,
                    output_worker,
                    ExecutionSupervisor {
                        execution_id,
                        active_state,
                        manager: supervisor_manager,
                        sink: supervisor_sink,
                        resumed_at,
                        outcome_policy,
                    },
                )
            });
        if let Err(error) = supervisor {
            self.remove(execution_id);
            event_sink.close();
            return Err(ExecutionStartError::Thread(error));
        }

        Ok(StartedExecution {
            execution_id,
            events,
            #[cfg(test)]
            temporary_directory,
        })
    }
}

/// Supervisor 线程独占的终态解释、Active 清理与事件发布上下文。
struct ExecutionSupervisor {
    /// 当前受管 Execution 的稳定身份。
    execution_id: ExecutionId,
    /// 取消线程与 Supervisor 共享的控制状态。
    active_state: Arc<Mutex<ExecutionControlState>>,
    /// 终态发布后移除 Active 索引的 Manager。
    manager: ExecutionManager,
    /// 当前 Session 的唯一事件生产端。
    sink: EventSink,
    /// 受管进程完成 Resume 的时间点。
    resumed_at: Instant,
    /// Preview/Run Hash 已绑定且由 Definition 校验的结果策略。
    outcome_policy: OutcomePolicy,
}

/// 持续把 OutputCapture 的有界 Batch 转入 Session 有界队列，直到两个 Pipe EOF。
fn forward_output(execution_id: ExecutionId, capture: OutputCapture, sink: EventSink) {
    while let Ok(batch) = capture.receiver().recv() {
        sink.push_output(execution_id, batch);
    }
    sink.note_dropped_bytes(capture.pending_dropped_bytes());
}

/// 等待根进程、关闭 Job 以清理遗留子孙、等待输出 EOF，并发布唯一终态。
fn supervise_execution(
    process: ManagedProcess,
    output_worker: thread::JoinHandle<()>,
    supervisor: ExecutionSupervisor,
) {
    let ExecutionSupervisor {
        execution_id,
        active_state,
        manager,
        sink,
        resumed_at,
        outcome_policy,
    } = supervisor;
    let wait_result = process.wait();
    let cleanup_result = process.terminate_job();
    let job_empty_result = process.wait_job_empty();
    let cancel_requested = {
        let mut state = lock_unpoisoned(&active_state);
        let requested = *state == ExecutionControlState::Cancelling;
        *state = ExecutionControlState::Terminated;
        requested
    };
    drop(process);
    let output_result = output_worker.join();
    let duration = resumed_at.elapsed();

    let terminal = match (wait_result, cleanup_result, job_empty_result, output_result) {
        (Ok(exit_code), Ok(()), Ok(()), Ok(()))
            if cancel_requested && exit_code == CMDBOX_CANCEL_EXIT_CODE =>
        {
            cancelled_terminal(execution_id, duration)
        }
        (Ok(exit_code), Ok(()), Ok(()), Ok(())) => {
            finished_terminal(execution_id, exit_code, duration, &outcome_policy)
        }
        (Err(error), _, _, _) => failed_terminal(execution_id, error.to_string(), duration),
        (Ok(_), Err(error), _, _) => failed_terminal(execution_id, error.to_string(), duration),
        (Ok(_), Ok(()), Err(error), _) => {
            failed_terminal(execution_id, error.to_string(), duration)
        }
        (Ok(_), Ok(()), Ok(()), Err(_)) => failed_terminal(
            execution_id,
            "Execution 输出转发线程异常退出".to_owned(),
            duration,
        ),
    };
    sink.push_terminal(terminal);
    manager.remove(execution_id);
}

/// 构造自然完成终态，并只用已验证 Policy 解释原始 Exit Code。
fn finished_terminal(
    execution_id: ExecutionId,
    exit_code: u32,
    duration: Duration,
    outcome_policy: &OutcomePolicy,
) -> ExecutionEvent {
    ExecutionEvent::Finished {
        execution_id,
        exit_code,
        outcome: outcome_policy.interpret_exit_code(exit_code),
        duration,
        dropped_output_bytes: 0,
    }
}

/// 构造已确认整树终止的取消终态，并固定没有业务 Outcome。
fn cancelled_terminal(execution_id: ExecutionId, duration: Duration) -> ExecutionEvent {
    ExecutionEvent::Cancelled {
        execution_id,
        outcome: Outcome::None,
        duration,
        dropped_output_bytes: 0,
    }
}

/// 构造 Core 内部失败终态，并防止其被误解释为命令业务 Failure。
fn failed_terminal(
    execution_id: ExecutionId,
    message: String,
    duration: Duration,
) -> ExecutionEvent {
    ExecutionEvent::Failed {
        execution_id,
        message,
        outcome: Outcome::None,
        duration,
        dropped_output_bytes: 0,
    }
}

/// 统计 Batch 自身文本和此前已声明丢弃字节，用于跨队列传递丢弃信息。
fn batch_total_bytes(batch: &OutputBatch) -> u64 {
    batch
        .fragments
        .iter()
        .map(|fragment| fragment.text.len() as u64)
        .sum::<u64>()
        .saturating_add(batch.dropped_bytes_before)
}

#[cfg(test)]
mod tests {
    //! 固定脚本 Session 的真实 Windows 生命周期、输出、取消和清理测试。

    use std::sync::mpsc::{RecvTimeoutError, TryRecvError};
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject};

    use super::{
        cancelled_terminal, failed_terminal, ExecutionEvent, ExecutionManager, StartedExecution,
    };
    use crate::execution::manager::ActiveExecutionState;
    use crate::execution::outcome::{ExitCodeRange, Outcome, OutcomePolicy};
    use crate::execution::output::OutputStream;
    use crate::execution::planner::{
        verified_windows_powershell_for_test, verified_windows_powershell_with_policy_for_test,
        VerifiedExecution,
    };

    /// 以方法类型证明 Session 唯一启动入口只接受 Planner 授权值。
    #[test]
    fn accepts_only_verified_execution_through_start_boundary() {
        let boundary: fn(
            &ExecutionManager,
            VerifiedExecution,
        ) -> Result<StartedExecution, super::ExecutionStartError> = ExecutionManager::start;
        let _ = boundary;
    }

    /// 验证取消和 Core 内部失败的集中构造器始终发布 `none`。
    #[test]
    fn keeps_cancelled_and_internal_failed_outcomes_none() {
        let execution_id = uuid::Uuid::new_v4();
        assert!(matches!(
            cancelled_terminal(execution_id, Duration::ZERO),
            ExecutionEvent::Cancelled {
                outcome: Outcome::None,
                ..
            }
        ));
        assert!(matches!(
            failed_terminal(execution_id, "internal".to_owned(), Duration::ZERO),
            ExecutionEvent::Failed {
                outcome: Outcome::None,
                ..
            }
        ));
    }

    /// 把无害测试脚本包装为仅在测试构建存在的 Planner 授权值。
    fn verified_test_script(script: &str) -> VerifiedExecution {
        verified_windows_powershell_for_test(script, std::env::temp_dir())
    }

    /// 在统一截止时间内读取到唯一终态，并返回包括 Started/Output 在内的完整事件序列。
    fn collect_until_terminal(started: &StartedExecution) -> Vec<ExecutionEvent> {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut events = Vec::new();
        while Instant::now() < deadline {
            match started.events.recv_timeout(Duration::from_millis(250)) {
                Ok(event) => {
                    let terminal = event.is_terminal();
                    events.push(event);
                    if terminal {
                        return events;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        panic!("应在截止时间内收到 Execution 终态，实际事件：{events:?}");
    }

    /// 等待 Supervisor 在终态发布后完成 Active 索引移除，避免测试依赖线程调度时机。
    fn wait_until_removed(manager: &ExecutionManager, execution_id: uuid::Uuid) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if !manager
                .active_snapshot()
                .iter()
                .any(|active| active.execution_id == execution_id)
            {
                return;
            }
            std::thread::yield_now();
        }
        panic!("终态发布后应移除 Active Execution：{execution_id}");
    }

    /// 在收到 Session 终态的同一时刻断言指定子进程已经退出。
    fn assert_process_already_exited(pid: u32) {
        const SYNCHRONIZE: u32 = 0x0010_0000;
        // SAFETY: 只申请等待权限，不读取或修改目标进程；成功句柄在本函数关闭一次。
        let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            return;
        }
        // SAFETY: Handle 有效；零超时只读取当前 signaled 状态，随后关闭句柄。
        let wait_result = unsafe { WaitForSingleObject(handle, 0) };
        unsafe {
            CloseHandle(handle);
        }
        assert_eq!(wait_result, WAIT_OBJECT_0, "终态发布时 PID {pid} 仍未退出");
    }

    /// 验证极短脚本不会丢 Started/Output/Finished，且终态前移除 Active 并清理 Artifact。
    #[test]
    fn runs_short_script_with_ordered_lifecycle_output_and_cleanup() {
        let manager = ExecutionManager::new();
        let script = "$utf8 = New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding = $utf8; [Console]::Out.Write('标准输出中文'); [Console]::Error.Write('标准错误中文')";
        let started = manager
            .start(verified_test_script(script))
            .expect("应启动固定 PowerShell Session");
        let execution_id = started.execution_id;
        let temporary_directory = started.temporary_directory().to_path_buf();
        assert!(manager
            .active_snapshot()
            .iter()
            .any(|active| active.execution_id == execution_id));

        let events = collect_until_terminal(&started);
        assert!(matches!(
            events.first(),
            Some(ExecutionEvent::Started {
                execution_id: observed,
                ..
            }) if *observed == execution_id
        ));
        assert!(matches!(
            events.last(),
            Some(ExecutionEvent::Finished {
                execution_id: observed,
                exit_code: 0,
                outcome: Outcome::Success,
                ..
            }) if *observed == execution_id
        ));
        let mut stdout = String::new();
        let mut stderr = String::new();
        for event in &events {
            if let ExecutionEvent::Output { batch, .. } = event {
                for fragment in &batch.fragments {
                    match fragment.stream {
                        OutputStream::Stdout => stdout.push_str(&fragment.text),
                        OutputStream::Stderr => stderr.push_str(&fragment.text),
                    }
                }
            }
        }
        assert!(stdout.contains("标准输出中文"), "实际 stdout：{stdout:?}");
        assert!(stderr.contains("标准错误中文"), "实际 stderr：{stderr:?}");
        wait_until_removed(&manager, execution_id);
        assert!(!temporary_directory.exists(), "终态前应清理当前 Artifact");
        assert_eq!(started.events.try_recv(), Err(TryRecvError::Disconnected));
    }

    /// 验证非零 PowerShell Exit Code 原样进入 Finished，不被解释为后端失败。
    #[test]
    fn preserves_non_zero_exit_code_as_finished() {
        let manager = ExecutionManager::new();
        let started = manager
            .start(verified_test_script("exit 7"))
            .expect("应启动非零退出脚本");
        let events = collect_until_terminal(&started);
        assert!(matches!(
            events.last(),
            Some(ExecutionEvent::Finished {
                exit_code: 7,
                outcome: Outcome::Failure,
                ..
            })
        ));
    }

    /// 验证特殊 Policy 经完整受管进程 Supervisor 路径解释真实非零 Exit Code。
    #[test]
    fn carries_special_policy_through_real_natural_supervisor_terminal() {
        let special = OutcomePolicy::exit_code(
            1,
            vec![ExitCodeRange { start: 0, end: 1 }],
            vec![ExitCodeRange { start: 2, end: 7 }],
        );
        let scenarios = [
            (1, Outcome::Success),
            (3, Outcome::Warning),
            (8, Outcome::Failure),
        ];

        for (exit_code, expected_outcome) in scenarios {
            let manager = ExecutionManager::new();
            let verified = verified_windows_powershell_with_policy_for_test(
                &format!("exit {exit_code}"),
                std::env::temp_dir(),
                special.clone(),
            );
            let started = manager
                .start(verified)
                .expect("安全特殊 Exit Code 脚本应启动");
            let events = collect_until_terminal(&started);
            assert!(matches!(
                events.last(),
                Some(ExecutionEvent::Finished {
                    exit_code: observed_exit,
                    outcome,
                    ..
                }) if *observed_exit == exit_code && *outcome == expected_outcome
            ));
            wait_until_removed(&manager, started.execution_id);
        }
    }

    /// 验证根 PowerShell 自然退出时，Session 会终止仍留在 Job 中的子孙并完成 Pipe EOF。
    #[test]
    fn natural_root_exit_cleans_remaining_job_descendants() {
        let manager = ExecutionManager::new();
        let script = "$child = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList @('-NoLogo', '-NoProfile', '-NonInteractive', '-Command', 'Start-Sleep -Seconds 5') -PassThru; [Console]::Out.WriteLine(\"child-pid:$($child.Id)\"); exit 0";
        let started = manager
            .start(verified_test_script(script))
            .expect("应启动会创建子进程的脚本");
        let events = collect_until_terminal(&started);
        assert!(matches!(
            events.last(),
            Some(ExecutionEvent::Finished { exit_code: 0, .. })
        ));
        let stdout = events
            .iter()
            .filter_map(|event| match event {
                ExecutionEvent::Output { batch, .. } => Some(
                    batch
                        .fragments
                        .iter()
                        .filter(|fragment| fragment.stream == OutputStream::Stdout)
                        .map(|fragment| fragment.text.as_str())
                        .collect::<String>(),
                ),
                _ => None,
            })
            .collect::<String>();
        let child_pid = stdout
            .split("child-pid:")
            .nth(1)
            .and_then(|value| value.lines().next())
            .and_then(|value| value.trim().parse::<u32>().ok())
            .expect("根进程应输出无害短等待子进程 PID");
        assert_process_already_exited(child_pid);
    }

    /// 验证首次取消推进 Cancelling，重复取消幂等，确认 Job 结束后只发布一个 Cancelled。
    #[test]
    fn cancels_job_idempotently_and_removes_active_execution() {
        let manager = ExecutionManager::new();
        let started = manager
            .start(verified_test_script("Start-Sleep -Seconds 5"))
            .expect("应启动长任务");
        let first = manager
            .cancel(started.execution_id)
            .expect("首次取消应请求终止 Job");
        assert!(first.accepted);
        assert_eq!(first.state, Some(ActiveExecutionState::Cancelling));
        let second = manager
            .cancel(started.execution_id)
            .expect("重复取消应返回稳定状态");
        assert!(!second.accepted);
        assert_eq!(second.state, Some(ActiveExecutionState::Cancelling));

        let events = collect_until_terminal(&started);
        assert_eq!(events.iter().filter(|event| event.is_terminal()).count(), 1);
        assert!(matches!(
            events.last(),
            Some(ExecutionEvent::Cancelled {
                outcome: Outcome::None,
                ..
            })
        ));
        wait_until_removed(&manager, started.execution_id);
        let after_terminal = manager
            .cancel(started.execution_id)
            .expect("终态后取消应稳定返回不存在");
        assert!(!after_terminal.accepted);
        assert_eq!(after_terminal.state, None);
    }

    /// 验证调用方完全不消费时事件队列保持有界，PowerShell 仍结束并在终态报告丢弃。
    #[test]
    fn slow_session_consumer_stays_bounded_and_does_not_block_process() {
        let manager = ExecutionManager::new();
        let script = "1..200000 | ForEach-Object { [Console]::Out.WriteLine('0123456789abcdef') }";
        let started_at = Instant::now();
        let started = manager
            .start(verified_test_script(script))
            .expect("应启动高频输出脚本");
        assert!(
            started.events.wait_until_closed(Duration::from_secs(15)),
            "零消费时外部进程与输出线程仍应在期限内结束"
        );
        assert!(started.events.pending_len() <= super::SESSION_EVENT_CAPACITY);
        let events = collect_until_terminal(&started);
        assert!(started_at.elapsed() < Duration::from_secs(15));
        let terminal_dropped = match events.last() {
            Some(ExecutionEvent::Finished {
                dropped_output_bytes,
                ..
            }) => *dropped_output_bytes,
            other => panic!("应自然结束并报告丢弃，实际：{other:?}"),
        };
        let batch_dropped = events
            .iter()
            .filter_map(|event| match event {
                ExecutionEvent::Output { batch, .. } => Some(batch.dropped_bytes_before),
                _ => None,
            })
            .sum::<u64>();
        assert!(terminal_dropped.saturating_add(batch_dropped) > 0);
    }

    /// 验证取消与自然退出接近时只产生一个终态，且终态观测后不再接受取消。
    #[test]
    fn cancel_and_natural_exit_race_has_one_terminal() {
        for delay in [0_u64, 30, 150, 400] {
            let manager = ExecutionManager::new();
            let started = manager
                .start(verified_test_script("Start-Sleep -Milliseconds 80; exit 9"))
                .expect("应启动竞态脚本");
            std::thread::sleep(Duration::from_millis(delay));
            let cancel = manager
                .cancel(started.execution_id)
                .expect("竞态取消调用不应失败");
            let events = collect_until_terminal(&started);
            assert_eq!(events.iter().filter(|event| event.is_terminal()).count(), 1);
            assert!(matches!(
                events.last(),
                Some(ExecutionEvent::Cancelled { .. })
                    | Some(ExecutionEvent::Finished { exit_code: 9, .. })
            ));
            if !cancel.accepted {
                assert!(matches!(
                    cancel.state,
                    None | Some(ActiveExecutionState::Cancelling)
                ));
            }
            wait_until_removed(&manager, started.execution_id);
        }
    }
}
