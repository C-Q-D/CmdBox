//! 固定 PowerShell 验收任务的 Tauri IPC 适配层。
//!
//! React 只能启动本文件内置的无破坏诊断脚本，并按 UUID 请求取消对应 Execution Job。
//! Session 仍由 Rust `ExecutionManager` 持有；Channel 断开只停止 UI 转发，不改变任务结果。

use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;
use uuid::Uuid;

use crate::execution::manager::{ActiveExecutionState, ExecutionManager};
use crate::execution::output::{OutputBatch, OutputStream};
use crate::execution::session::{ExecutionEvent, ExecutionEventReceiver, ExecutionStartError};

/// 固定子进程在 Windows 命令行中的诊断标记，仅供真实宿主验收进程树清理。
pub const DIAGNOSTIC_CHILD_MARKER: &str = "CmdBox-CMD01-Diagnostic-Child";

/// 固定、无用户输入且无文件删除行为的 CMD-01 验收脚本。
const FIXED_EXECUTION_SCRIPT: &str = r#"
$utf8 = New-Object System.Text.UTF8Encoding($false)
[Console]::OutputEncoding = $utf8
[Console]::Error.WriteLine('CmdBox stderr 通道已连接')
[Console]::Out.WriteLine('<b>CmdBox output remains text</b>')
[Console]::Out.WriteLine('https://example.invalid/cmdbox-output')
[Console]::Out.WriteLine(([char]27) + '[31mANSI remains text' + ([char]27) + '[0m')

$childCommand = "& { `$marker = 'CmdBox-CMD01-Diagnostic-Child'; Start-Sleep -Seconds 120 }"
$child = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList @(
    '-NoLogo',
    '-NoProfile',
    '-NonInteractive',
    '-Command',
    $childCommand
) -PassThru
[Console]::Out.WriteLine('受管子进程已启动')

1..8 | ForEach-Object {
    [Console]::Out.WriteLine("验收任务进度 $_/8")
    if ($_ % 2 -eq 0) {
        [Console]::Error.WriteLine("验收任务 stderr $_/8")
    }
    Start-Sleep -Seconds 1
}

[Console]::Out.WriteLine('验收任务自然结束')
exit 0
"#;

/// IPC 调用失败时返回的稳定错误对象。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    /// 供前端稳定分支处理的错误码。
    pub code: &'static str,
    /// 面向用户或开发者的中文错误说明。
    pub message: String,
}

/// 固定任务启动成功后的最小响应。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartFixedExecutionResponse {
    /// 新建 Execution 的 UUID 字符串。
    pub execution_id: String,
}

/// 取消调用返回的非终态事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelExecutionResponse {
    /// 本次调用是否首次接受取消请求。
    pub accepted: bool,
    /// Execution 当前可观察状态；不存在或已终止时为 `None`。
    pub state: Option<IpcActiveExecutionState>,
}

/// 可通过 IPC 观察的 Active Execution 状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum IpcActiveExecutionState {
    /// Execution 正在运行。
    Running,
    /// 已接受取消，等待 Job 清空。
    Cancelling,
}

/// 输出文本来自哪个标准流。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum IpcOutputStream {
    /// 标准输出。
    Stdout,
    /// 标准错误。
    Stderr,
}

/// 一个按 Output Coordinator 观察顺序生成的纯文本片段。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IpcOutputFragment {
    /// Output Coordinator 分配的片段级顺序，不与事件级顺序混用。
    pub fragment_sequence: u64,
    /// 片段所属标准流。
    pub stream: IpcOutputStream,
    /// 已在 Rust 侧增量解码的不可信纯文本。
    pub text: String,
}

/// 专属 Tauri Channel 上的完整事件流。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event",
    content = "data"
)]
pub enum ExecutionStreamEvent {
    /// Session 已登记且受管进程即将恢复。
    Started {
        /// 当前 Execution UUID。
        execution_id: String,
        /// IPC 转发器分配的事件级顺序。
        sequence: u64,
    },
    /// 一个有界 Output Batch。
    Output {
        /// 当前 Execution UUID。
        execution_id: String,
        /// IPC 转发器分配的事件级顺序。
        sequence: u64,
        /// 保持协调器观察顺序的纯文本片段。
        fragments: Vec<IpcOutputFragment>,
        /// 当前 Batch 之前因有界队列压力被丢弃的字节数。
        dropped_bytes_before: u64,
    },
    /// 根进程自然结束且 Job 已清空。
    Finished {
        /// 当前 Execution UUID。
        execution_id: String,
        /// IPC 转发器分配的事件级顺序。
        sequence: u64,
        /// Windows PowerShell 原始 Exit Code。
        exit_code: u32,
        /// Rust Core 从 Resume 到终态的毫秒数。
        duration_ms: u64,
        /// 尚未随 Output Batch 报告的丢弃字节数。
        dropped_output_bytes: u64,
    },
    /// 取消已被接受并且整个 Job 已确认结束。
    Cancelled {
        /// 当前 Execution UUID。
        execution_id: String,
        /// IPC 转发器分配的事件级顺序。
        sequence: u64,
        /// Rust Core 从 Resume 到终态的毫秒数。
        duration_ms: u64,
        /// 尚未随 Output Batch 报告的丢弃字节数。
        dropped_output_bytes: u64,
    },
    /// Resume 后发生后端内部失败。
    Failed {
        /// 当前 Execution UUID。
        execution_id: String,
        /// IPC 转发器分配的事件级顺序。
        sequence: u64,
        /// Rust Core 返回的稳定失败说明。
        message: String,
        /// Rust Core 从 Resume 到终态的毫秒数。
        duration_ms: u64,
        /// 尚未随 Output Batch 报告的丢弃字节数。
        dropped_output_bytes: u64,
    },
}

/// 启动固定、无破坏性的 CMD-01 验收任务，并把事件绑定到调用方专属 Channel。
#[tauri::command]
pub fn start_fixed_execution(
    manager: State<'_, ExecutionManager>,
    on_event: Channel<ExecutionStreamEvent>,
) -> Result<StartFixedExecutionResponse, ApiError> {
    start_with_sender(
        manager.inner().clone(),
        FIXED_EXECUTION_SCRIPT,
        move |event| on_event.send(event).map_err(|_| ()),
    )
}

/// 按 Execution UUID 请求终止对应的整个 Windows Job；不接受 PID。
#[tauri::command]
pub fn cancel_execution(
    manager: State<'_, ExecutionManager>,
    execution_id: String,
) -> Result<CancelExecutionResponse, ApiError> {
    let execution_id = Uuid::parse_str(&execution_id).map_err(|_| ApiError {
        code: "VALIDATION_FAILED",
        message: "Execution ID 不是有效的 UUID".to_owned(),
    })?;
    let result = manager.cancel(execution_id).map_err(|_| ApiError {
        code: "CANCEL_FAILED",
        message: "无法终止当前 Execution".to_owned(),
    })?;

    Ok(CancelExecutionResponse {
        accepted: result.accepted,
        state: result.state.map(IpcActiveExecutionState::from),
    })
}

/// 启动固定脚本并创建独立 IPC 转发线程；测试可替换事件发送端而不构造 WebView。
fn start_with_sender<F>(
    manager: ExecutionManager,
    script: &str,
    sender: F,
) -> Result<StartFixedExecutionResponse, ApiError>
where
    F: Fn(ExecutionStreamEvent) -> Result<(), ()> + Send + 'static,
{
    let started = manager
        .start_fixed_powershell(script, &std::env::temp_dir())
        .map_err(public_start_error)?;
    let execution_id = started.execution_id;
    let forwarding_manager = manager.clone();
    if thread::Builder::new()
        .name(format!("cmdbox-ipc-forward-{execution_id}"))
        .spawn(move || forward_events(started.events, sender))
        .is_err()
    {
        // 转发器无法建立时调用方不可能观察任务，因此立即请求整树清理并返回启动失败。
        let _ = forwarding_manager.cancel(execution_id);
        return Err(ApiError {
            code: "EXECUTION_START_FAILED",
            message: "无法建立 Execution 事件通道".to_owned(),
        });
    }

    Ok(StartFixedExecutionResponse {
        execution_id: execution_id.to_string(),
    })
}

/// 把可能包含本机路径或系统错误的启动失败收敛为稳定公开错误。
fn public_start_error(error: ExecutionStartError) -> ApiError {
    match error {
        ExecutionStartError::Runner(_) => ApiError {
            code: "RUNNER_UNAVAILABLE",
            message: "系统 Windows PowerShell 不可用".to_owned(),
        },
        ExecutionStartError::Artifact(_) => ApiError {
            code: "ARTIFACT_PREPARATION_FAILED",
            message: "无法准备固定任务临时脚本".to_owned(),
        },
        ExecutionStartError::Process(_) => ApiError {
            code: "PROCESS_START_FAILED",
            message: "无法启动固定 PowerShell 任务".to_owned(),
        },
        ExecutionStartError::Thread(_) => ApiError {
            code: "EXECUTION_START_FAILED",
            message: "无法建立 Execution 后台任务".to_owned(),
        },
    }
}

/// 顺序消费 Session 事件并发送到专属 Channel；Channel 失败只结束当前 UI 转发。
fn forward_events<F>(events: ExecutionEventReceiver, sender: F)
where
    F: Fn(ExecutionStreamEvent) -> Result<(), ()>,
{
    let mut sequence = 0_u64;
    loop {
        match events.recv_timeout(Duration::from_secs(1)) {
            Ok(event) => {
                let terminal = event.is_terminal();
                let ipc_event = map_event(event, sequence);
                if sender(ipc_event).is_err() || terminal {
                    return;
                }
                sequence = sequence.saturating_add(1);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// 把 Rust Core 事件映射成不泄露 PID 的稳定 IPC 契约。
fn map_event(event: ExecutionEvent, sequence: u64) -> ExecutionStreamEvent {
    match event {
        ExecutionEvent::Started { execution_id, .. } => ExecutionStreamEvent::Started {
            execution_id: execution_id.to_string(),
            sequence,
        },
        ExecutionEvent::Output {
            execution_id,
            batch,
        } => map_output(execution_id.to_string(), sequence, batch),
        ExecutionEvent::Finished {
            execution_id,
            exit_code,
            duration,
            dropped_output_bytes,
        } => ExecutionStreamEvent::Finished {
            execution_id: execution_id.to_string(),
            sequence,
            exit_code,
            duration_ms: duration.as_millis() as u64,
            dropped_output_bytes,
        },
        ExecutionEvent::Cancelled {
            execution_id,
            duration,
            dropped_output_bytes,
        } => ExecutionStreamEvent::Cancelled {
            execution_id: execution_id.to_string(),
            sequence,
            duration_ms: duration.as_millis() as u64,
            dropped_output_bytes,
        },
        ExecutionEvent::Failed {
            execution_id,
            message,
            duration,
            dropped_output_bytes,
        } => ExecutionStreamEvent::Failed {
            execution_id: execution_id.to_string(),
            sequence,
            message,
            duration_ms: duration.as_millis() as u64,
            dropped_output_bytes,
        },
    }
}

/// 映射一个有界 Output Batch，并保留片段级顺序和流来源。
fn map_output(execution_id: String, sequence: u64, batch: OutputBatch) -> ExecutionStreamEvent {
    ExecutionStreamEvent::Output {
        execution_id,
        sequence,
        fragments: batch
            .fragments
            .into_iter()
            .map(|fragment| IpcOutputFragment {
                fragment_sequence: fragment.sequence,
                stream: match fragment.stream {
                    OutputStream::Stdout => IpcOutputStream::Stdout,
                    OutputStream::Stderr => IpcOutputStream::Stderr,
                },
                text: fragment.text,
            })
            .collect(),
        dropped_bytes_before: batch.dropped_bytes_before,
    }
}

/// 把 Core Active 状态映射为不包含进程实现细节的 IPC 状态。
impl From<ActiveExecutionState> for IpcActiveExecutionState {
    fn from(state: ActiveExecutionState) -> Self {
        match state {
            ActiveExecutionState::Running => Self::Running,
            ActiveExecutionState::Cancelling => Self::Cancelling,
        }
    }
}

#[cfg(test)]
mod tests {
    //! Typed IPC 的序列化边界、事件顺序和 Channel 故障隔离测试。

    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use super::{
        map_event, public_start_error, start_with_sender, ApiError, ExecutionStreamEvent,
        IpcOutputStream,
    };
    use crate::execution::artifact::{ArtifactError, ArtifactOperation};
    use crate::execution::manager::ExecutionManager;
    use crate::execution::output::{OutputBatch, OutputFragment, OutputStream};
    use crate::execution::session::{ExecutionEvent, ExecutionStartError};

    /// 验证 Started 不把后端 PID 带入 IPC，且事件级 sequence 可排序。
    #[test]
    fn maps_started_without_exposing_process_id() {
        let execution_id = uuid::Uuid::new_v4();
        let event = map_event(
            ExecutionEvent::Started {
                execution_id,
                process_id: 4242,
            },
            7,
        );

        assert_eq!(
            event,
            ExecutionStreamEvent::Started {
                execution_id: execution_id.to_string(),
                sequence: 7,
            }
        );
    }

    /// 验证事件级顺序与 Output Fragment 顺序使用不同字段并完整保留流来源。
    #[test]
    fn maps_output_batch_with_distinct_sequences() {
        let execution_id = uuid::Uuid::new_v4();
        let event = map_event(
            ExecutionEvent::Output {
                execution_id,
                batch: OutputBatch {
                    fragments: vec![OutputFragment {
                        sequence: 21,
                        stream: OutputStream::Stderr,
                        text: "错误文本".to_owned(),
                    }],
                    dropped_bytes_before: 128,
                },
            },
            3,
        );

        let ExecutionStreamEvent::Output {
            sequence,
            fragments,
            dropped_bytes_before,
            ..
        } = event
        else {
            panic!("应映射为 Output");
        };
        assert_eq!(sequence, 3);
        assert_eq!(fragments[0].fragment_sequence, 21);
        assert_eq!(fragments[0].stream, IpcOutputStream::Stderr);
        assert_eq!(dropped_bytes_before, 128);
    }

    /// 验证无效 UUID 得到稳定校验错误，不触达 Manager 取消入口。
    #[test]
    fn exposes_stable_invalid_execution_id_error() {
        let result = uuid::Uuid::parse_str("not-an-execution-id").map_err(|_| ApiError {
            code: "VALIDATION_FAILED",
            message: "Execution ID 不是有效的 UUID".to_owned(),
        });

        assert_eq!(
            result.expect_err("无效 UUID 应失败"),
            ApiError {
                code: "VALIDATION_FAILED",
                message: "Execution ID 不是有效的 UUID".to_owned(),
            }
        );
    }

    /// 验证包含本机临时路径的底层错误不会跨越 IPC 信任边界。
    #[test]
    fn redacts_local_paths_from_public_start_errors() {
        let private_path = std::path::PathBuf::from(r"C:\Users\Private\Temp\CmdBox\script.ps1");
        let error = public_start_error(ExecutionStartError::Artifact(ArtifactError::Io {
            operation: ArtifactOperation::CreateExecutionDirectory,
            path: private_path.clone(),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "private detail"),
        }));

        assert_eq!(error.code, "ARTIFACT_PREPARATION_FAILED");
        assert!(!error
            .message
            .contains(private_path.to_string_lossy().as_ref()));
        assert!(!error.message.contains("private detail"));
    }

    /// 验证 UI Channel 立即断开不会取消 Session，固定任务仍可自然运行到 Manager 清理。
    #[test]
    fn channel_failure_does_not_cancel_execution() {
        let manager = ExecutionManager::new();
        let sent_events = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&sent_events);
        let started = start_with_sender(
            manager.clone(),
            "[Console]::Out.WriteLine('started'); Start-Sleep -Milliseconds 700; exit 0",
            move |event| {
                observed.lock().expect("事件锁不应中毒").push(event);
                Err(())
            },
        )
        .expect("短任务应启动");

        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            manager.active_snapshot().len(),
            1,
            "Channel 失败不应取消任务"
        );

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !manager.active_snapshot().is_empty() {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(manager.active_snapshot().is_empty(), "任务应自然结束并清理");
        assert!(
            started.execution_id.parse::<uuid::Uuid>().is_ok(),
            "启动响应应返回 UUID"
        );
        assert_eq!(sent_events.lock().expect("事件锁不应中毒").len(), 1);
    }
}
