//! Command Block 读取、Preview、Run、事件与取消的窄 Tauri IPC 适配层。
//!
//! React 只能提交业务 ID、revision、结构化参数、Preview Hash 和专属 Channel。Planner 在
//! 一切启动副作用之前复验完整 Execution Spec；本层不接受脚本、可执行文件、PID 或参数旁路。

use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;
use uuid::Uuid;

use crate::execution::manager::{ActiveExecutionState, ExecutionManager};
use crate::execution::output::{OutputBatch, OutputStream};
use crate::execution::planner::{
    CommandBlockDetails, CommandBlockSummary, ExecutionPlanner, PlannerError, PlannerErrorCode,
    PreviewCommandRequest, PreviewCommandResponse, VerifyRunRequest,
};
use crate::execution::session::{ExecutionEvent, ExecutionEventReceiver, ExecutionStartError};

/// IPC 调用失败时返回的稳定错误对象。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    /// 供前端稳定分支处理的错误码。
    pub code: &'static str,
    /// 面向用户或开发者的中文错误说明。
    pub message: String,
    /// 参数校验错误直接关联的 Parameter key；其他错误省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub parameter_key: Option<String>,
    /// 参数或内部模板错误的稳定原因码；其他错误省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub detail_code: Option<String>,
}

impl ApiError {
    /// 创建不携带参数定位信息的稳定公开错误。
    fn simple(code: &'static str, message: &'static str) -> Self {
        Self {
            code,
            message: message.to_owned(),
            parameter_key: None,
            detail_code: None,
        }
    }
}

/// Command Block 启动成功后的最小响应。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase")]
pub struct RunCommandResponse {
    /// 新建 Execution 的 UUID 字符串。
    pub execution_id: String,
}

/// 取消调用返回的非终态事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase")]
pub struct CancelExecutionResponse {
    /// 本次调用是否首次接受取消请求。
    pub accepted: bool,
    /// Execution 当前可观察状态；不存在或已终止时为 `None`。
    pub state: Option<IpcActiveExecutionState>,
}

/// 可通过 IPC 观察的 Active Execution 状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase")]
pub enum IpcActiveExecutionState {
    /// Execution 正在运行。
    Running,
    /// 已接受取消，等待 Job 清空。
    Cancelling,
}

/// 输出文本来自哪个标准流。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase")]
pub enum IpcOutputStream {
    /// 标准输出。
    Stdout,
    /// 标准错误。
    Stderr,
}

/// 一个按 Output Coordinator 观察顺序生成的纯文本片段。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase")]
pub struct IpcOutputFragment {
    /// Output Coordinator 分配的片段级顺序，不与事件级顺序混用。
    #[cfg_attr(test, ts(type = "number"))]
    pub fragment_sequence: u64,
    /// 片段所属标准流。
    pub stream: IpcOutputStream,
    /// 已在 Rust 侧增量解码的不可信纯文本。
    pub text: String,
}

/// 专属 Tauri Channel 上的完整事件流。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
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
        #[cfg_attr(test, ts(type = "number"))]
        sequence: u64,
    },
    /// 一个有界 Output Batch。
    Output {
        /// 当前 Execution UUID。
        execution_id: String,
        /// IPC 转发器分配的事件级顺序。
        #[cfg_attr(test, ts(type = "number"))]
        sequence: u64,
        /// 保持协调器观察顺序的纯文本片段。
        fragments: Vec<IpcOutputFragment>,
        /// 当前 Batch 之前因有界队列压力被丢弃的字节数。
        #[cfg_attr(test, ts(type = "number"))]
        dropped_bytes_before: u64,
    },
    /// 根进程自然结束且 Job 已清空。
    Finished {
        /// 当前 Execution UUID。
        execution_id: String,
        /// IPC 转发器分配的事件级顺序。
        #[cfg_attr(test, ts(type = "number"))]
        sequence: u64,
        /// Runner 根进程的原始 Exit Code。
        exit_code: u32,
        /// Rust Core 从 Resume 到终态的毫秒数。
        #[cfg_attr(test, ts(type = "number"))]
        duration_ms: u64,
        /// 尚未随 Output Batch 报告的丢弃字节数。
        #[cfg_attr(test, ts(type = "number"))]
        dropped_output_bytes: u64,
    },
    /// 取消已被接受并且整个 Job 已确认结束。
    Cancelled {
        /// 当前 Execution UUID。
        execution_id: String,
        /// IPC 转发器分配的事件级顺序。
        #[cfg_attr(test, ts(type = "number"))]
        sequence: u64,
        /// Rust Core 从 Resume 到终态的毫秒数。
        #[cfg_attr(test, ts(type = "number"))]
        duration_ms: u64,
        /// 尚未随 Output Batch 报告的丢弃字节数。
        #[cfg_attr(test, ts(type = "number"))]
        dropped_output_bytes: u64,
    },
    /// Resume 后发生后端内部失败。
    Failed {
        /// 当前 Execution UUID。
        execution_id: String,
        /// IPC 转发器分配的事件级顺序。
        #[cfg_attr(test, ts(type = "number"))]
        sequence: u64,
        /// Rust Core 返回的稳定失败说明。
        message: String,
        /// Rust Core 从 Resume 到终态的毫秒数。
        #[cfg_attr(test, ts(type = "number"))]
        duration_ms: u64,
        /// 尚未随 Output Batch 报告的丢弃字节数。
        #[cfg_attr(test, ts(type = "number"))]
        dropped_output_bytes: u64,
    },
}

/// 按稳定顺序列出 Rust Core 当前提供的 Command Block 公开摘要。
#[tauri::command]
pub fn list_command_blocks(planner: State<'_, ExecutionPlanner>) -> Vec<CommandBlockSummary> {
    planner.list_command_blocks()
}

/// 读取一个 Command Block 的公开详情；内部模板与启动配置不会跨越 IPC。
#[tauri::command]
pub fn get_command_block(
    planner: State<'_, ExecutionPlanner>,
    command_block_id: String,
) -> Result<CommandBlockDetails, ApiError> {
    planner
        .get_command_block(&command_block_id)
        .map_err(public_planner_error)
}

/// 由 Rust Core 校验参数并生成绑定完整 Execution Spec 的可信 Preview。
#[tauri::command]
pub fn preview_command_block(
    planner: State<'_, ExecutionPlanner>,
    request: PreviewCommandRequest,
) -> Result<PreviewCommandResponse, ApiError> {
    planner.preview(&request).map_err(public_planner_error)
}

/// 复验当前 Preview 后启动受管 Execution，并绑定调用方专属事件 Channel。
#[tauri::command]
pub fn run_command_block(
    planner: State<'_, ExecutionPlanner>,
    manager: State<'_, ExecutionManager>,
    request: VerifyRunRequest,
    on_event: Channel<ExecutionStreamEvent>,
) -> Result<RunCommandResponse, ApiError> {
    run_with_sender(
        planner.inner(),
        manager.inner().clone(),
        &request,
        move |event| on_event.send(event).map_err(|_| ()),
    )
}

/// 按 Execution UUID 请求终止对应的整个 Windows Job；不接受 PID。
#[tauri::command]
pub fn cancel_execution(
    manager: State<'_, ExecutionManager>,
    execution_id: String,
) -> Result<CancelExecutionResponse, ApiError> {
    let execution_id = Uuid::parse_str(&execution_id)
        .map_err(|_| ApiError::simple("VALIDATION_FAILED", "Execution ID 不是有效的 UUID"))?;
    let result = manager
        .cancel(execution_id)
        .map_err(|_| ApiError::simple("CANCEL_FAILED", "无法终止当前 Execution"))?;

    Ok(CancelExecutionResponse {
        accepted: result.accepted,
        state: result.state.map(IpcActiveExecutionState::from),
    })
}

/// 严格按复验、启动、事件转发的顺序执行；测试可替换发送端而不构造 WebView。
fn run_with_sender<F>(
    planner: &ExecutionPlanner,
    manager: ExecutionManager,
    request: &VerifyRunRequest,
    sender: F,
) -> Result<RunCommandResponse, ApiError>
where
    F: Fn(ExecutionStreamEvent) -> Result<(), ()> + Send + 'static,
{
    let verified = planner.verify_run(request).map_err(public_planner_error)?;
    let started = manager.start(verified).map_err(public_start_error)?;
    let execution_id = started.execution_id;
    let forwarding_manager = manager.clone();
    if thread::Builder::new()
        .name(format!("cmdbox-ipc-forward-{execution_id}"))
        .spawn(move || forward_events(started.events, sender))
        .is_err()
    {
        // 转发器无法建立时调用方不可能观察任务，因此立即请求整树清理并返回启动失败。
        let _ = forwarding_manager.cancel(execution_id);
        return Err(ApiError::simple(
            "EXECUTION_START_FAILED",
            "无法建立 Execution 事件通道",
        ));
    }

    Ok(RunCommandResponse {
        execution_id: execution_id.to_string(),
    })
}

/// 把 Planner 内部错误映射为不泄露模板、参数原值或本机路径的稳定公开错误。
fn public_planner_error(error: PlannerError) -> ApiError {
    let message = match error.code {
        PlannerErrorCode::CommandBlockNotFound => "未找到指定的 Command Block",
        PlannerErrorCode::RevisionConflict => "Command Block 已更新，请重新载入",
        PlannerErrorCode::ValidationFailed => "参数未通过校验",
        PlannerErrorCode::InvalidTemplate => "Command Block 模板无效",
        PlannerErrorCode::UnsupportedRunner => "当前 Runner 尚不支持",
        PlannerErrorCode::RunnerUnavailable => "系统 Runner 不可用",
        PlannerErrorCode::InternalContract => "Command Block 内部契约无效",
        PlannerErrorCode::StalePreview => "Preview 已失效，请重新生成",
    };
    ApiError {
        code: error.code.as_str(),
        message: message.to_owned(),
        parameter_key: error.parameter_key,
        detail_code: error.detail_code,
    }
}

/// 把可能包含本机路径或系统错误的启动失败收敛为稳定公开错误。
fn public_start_error(error: ExecutionStartError) -> ApiError {
    match error {
        ExecutionStartError::Artifact(_) => {
            ApiError::simple("ARTIFACT_PREPARATION_FAILED", "无法准备 Execution 临时脚本")
        }
        ExecutionStartError::Process(_) => {
            ApiError::simple("PROCESS_START_FAILED", "无法启动 Execution 进程")
        }
        ExecutionStartError::Thread(_) => {
            ApiError::simple("EXECUTION_START_FAILED", "无法建立 Execution 后台任务")
        }
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
    //! Typed IPC 字段白名单、真实 Tauri invoke、启动前拒绝与 Channel 故障隔离测试。

    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use serde_json::Value;

    use super::{
        map_event, public_planner_error, public_start_error, run_with_sender, ApiError,
        ExecutionStreamEvent, IpcOutputStream, RunCommandResponse,
    };
    use crate::execution::artifact::{ArtifactError, ArtifactOperation};
    use crate::execution::command::{CMD_PARAMETER_ECHO_ID, POWERSHELL_PARAMETER_ECHO_ID};
    use crate::execution::manager::ExecutionManager;
    use crate::execution::output::{OutputBatch, OutputFragment, OutputStream};
    use crate::execution::parameter::ParameterValue;
    use crate::execution::planner::{
        verified_windows_powershell_for_test, ExecutionPlanner, PlannerError, PlannerErrorCode,
        PreviewCommandRequest, VerifyRunRequest,
    };
    use crate::execution::session::{ExecutionEvent, ExecutionStartError};

    /// 返回固定 PowerShell Built-in 可接受的完整六类参数。
    fn valid_values(enabled: bool) -> BTreeMap<String, ParameterValue> {
        valid_values_with_text(enabled, "中文 空格 user's value")
    }

    /// 返回指定 Text 与固定其余五类值，供两个 Runner 的同接口回归复用。
    fn valid_values_with_text(enabled: bool, text: &str) -> BTreeMap<String, ParameterValue> {
        let temporary = std::env::temp_dir().to_string_lossy().into_owned();
        let current = std::env::current_dir()
            .expect("测试当前目录应存在")
            .to_string_lossy()
            .into_owned();
        BTreeMap::from([
            ("text".to_owned(), ParameterValue::Text(text.to_owned())),
            ("count".to_owned(), ParameterValue::Number(4.0)),
            ("enabled".to_owned(), ParameterValue::Boolean(enabled)),
            (
                "mode".to_owned(),
                ParameterValue::Text("detailed".to_owned()),
            ),
            ("folder".to_owned(), ParameterValue::Text(temporary.clone())),
            (
                "folders".to_owned(),
                ParameterValue::Array(vec![
                    ParameterValue::Text(temporary),
                    ParameterValue::Text(current),
                ]),
            ),
        ])
    }

    /// 生成当前 PowerShell Built-in 的 Preview 与后续 Run 请求。
    fn preview_and_run_request(
        planner: &ExecutionPlanner,
        enabled: bool,
    ) -> (
        crate::execution::planner::PreviewCommandResponse,
        VerifyRunRequest,
    ) {
        preview_and_run_request_for(
            planner,
            POWERSHELL_PARAMETER_ECHO_ID,
            enabled,
            "中文 空格 user's value",
        )
    }

    /// 生成指定参数回显 Built-in 的 Preview 与后续同 Hash Run 请求。
    fn preview_and_run_request_for(
        planner: &ExecutionPlanner,
        command_block_id: &str,
        enabled: bool,
        text: &str,
    ) -> (
        crate::execution::planner::PreviewCommandResponse,
        VerifyRunRequest,
    ) {
        let values = valid_values_with_text(enabled, text);
        let preview = planner
            .preview(&PreviewCommandRequest {
                command_block_id: command_block_id.to_owned(),
                expected_revision: 1,
                parameter_values: values.clone(),
            })
            .expect("参数回显 Built-in Preview 应成功");
        let request = VerifyRunRequest {
            command_block_id: command_block_id.to_owned(),
            expected_revision: preview.revision,
            parameter_values: values,
            execution_spec_hash: preview.execution_spec_hash.clone(),
        };
        (preview, request)
    }

    /// 返回 `%TEMP%\CmdBox` 当前直属 Execution 目录名集合，不递归读取目录内容。
    fn temporary_execution_directories() -> BTreeSet<String> {
        let root = std::env::temp_dir().join("CmdBox");
        let Ok(entries) = fs::read_dir(root) else {
            return BTreeSet::new();
        };
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                entry
                    .file_type()
                    .ok()
                    .filter(|kind| kind.is_dir())
                    .map(|_| entry.file_name().to_string_lossy().into_owned())
            })
            .collect()
    }

    /// 返回目录直属项名称集合，用于证明参数回显命令不修改目标内容。
    fn direct_directory_entries(path: &std::path::Path) -> BTreeSet<String> {
        fs::read_dir(path)
            .expect("测试目录应可读取")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect()
    }

    /// 递归断言公开 JSON 对象没有执行旁路字段。
    fn assert_no_forbidden_keys(value: &Value) {
        const FORBIDDEN: [&str; 7] = [
            "template",
            "script",
            "executable",
            "pid",
            "cwd",
            "workingDirectory",
            "environment",
        ];
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    assert!(!FORBIDDEN.contains(&key.as_str()), "发现旁路字段：{key}");
                    assert_no_forbidden_keys(child);
                }
            }
            Value::Array(values) => {
                for child in values {
                    assert_no_forbidden_keys(child);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }

    /// 等待测试发送端收到唯一终态并返回当前事件快照。
    fn wait_for_terminal(
        events: &Arc<Mutex<Vec<ExecutionStreamEvent>>>,
    ) -> Vec<ExecutionStreamEvent> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let snapshot = events.lock().expect("事件锁不应中毒").clone();
            if snapshot.iter().any(is_ipc_terminal) {
                return snapshot;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("测试发送端应在截止时间内收到终态");
    }

    /// 返回 IPC 事件是否为 Finished、Cancelled 或 Failed 唯一终态。
    fn is_ipc_terminal(event: &ExecutionStreamEvent) -> bool {
        matches!(
            event,
            ExecutionStreamEvent::Finished { .. }
                | ExecutionStreamEvent::Cancelled { .. }
                | ExecutionStreamEvent::Failed { .. }
        )
    }

    /// 返回 IPC 转发器分配的事件级顺序。
    fn ipc_sequence(event: &ExecutionStreamEvent) -> u64 {
        match event {
            ExecutionStreamEvent::Started { sequence, .. }
            | ExecutionStreamEvent::Output { sequence, .. }
            | ExecutionStreamEvent::Finished { sequence, .. }
            | ExecutionStreamEvent::Cancelled { sequence, .. }
            | ExecutionStreamEvent::Failed { sequence, .. } => *sequence,
        }
    }

    /// 返回每条 IPC 事件携带的 Execution UUID 文本。
    fn ipc_execution_id(event: &ExecutionStreamEvent) -> &str {
        match event {
            ExecutionStreamEvent::Started { execution_id, .. }
            | ExecutionStreamEvent::Output { execution_id, .. }
            | ExecutionStreamEvent::Finished { execution_id, .. }
            | ExecutionStreamEvent::Cancelled { execution_id, .. }
            | ExecutionStreamEvent::Failed { execution_id, .. } => execution_id,
        }
    }

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
        let json = serde_json::to_value(event).expect("Started 应可序列化");
        assert_no_forbidden_keys(&json);
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

    /// 验证公开 DTO 精确白名单且 Run Request 只含业务身份、值和 Preview Hash。
    #[test]
    fn exposes_only_command_block_ipc_field_whitelists() {
        let planner = ExecutionPlanner::new();
        let (preview, run_request) = preview_and_run_request(&planner, true);
        let summary =
            serde_json::to_value(&planner.list_command_blocks()[0]).expect("Summary 应可序列化");
        let details = serde_json::to_value(
            planner
                .get_command_block(POWERSHELL_PARAMETER_ECHO_ID)
                .expect("Details 应存在"),
        )
        .expect("Details 应可序列化");
        let preview_request = serde_json::to_value(PreviewCommandRequest {
            command_block_id: POWERSHELL_PARAMETER_ECHO_ID.to_owned(),
            expected_revision: 1,
            parameter_values: valid_values(true),
        })
        .expect("Preview Request 应可序列化");
        let preview_response = serde_json::to_value(preview).expect("Preview 应可序列化");
        let run_request = serde_json::to_value(run_request).expect("Run Request 应可序列化");
        let run_response = serde_json::to_value(RunCommandResponse {
            execution_id: uuid::Uuid::new_v4().to_string(),
        })
        .expect("Run Response 应可序列化");

        assert_eq!(
            preview_request
                .as_object()
                .expect("Preview Request 应为对象")
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["commandBlockId", "expectedRevision", "parameterValues"])
        );
        assert_eq!(
            run_request
                .as_object()
                .expect("Run Request 应为对象")
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "commandBlockId",
                "executionSpecHash",
                "expectedRevision",
                "parameterValues",
            ])
        );
        assert_eq!(
            run_response
                .as_object()
                .expect("Run Response 应为对象")
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["executionId"])
        );
        for value in [
            summary,
            details,
            preview_request,
            preview_response,
            run_request,
            run_response,
        ] {
            assert_no_forbidden_keys(&value);
        }
    }

    /// 验证全部 Planner 错误映射为稳定公开码和固定消息，参数定位只按白名单透传。
    #[test]
    fn maps_planner_errors_without_private_details() {
        let cases = [
            (
                PlannerErrorCode::CommandBlockNotFound,
                "未找到指定的 Command Block",
            ),
            (
                PlannerErrorCode::RevisionConflict,
                "Command Block 已更新，请重新载入",
            ),
            (PlannerErrorCode::ValidationFailed, "参数未通过校验"),
            (PlannerErrorCode::InvalidTemplate, "Command Block 模板无效"),
            (PlannerErrorCode::UnsupportedRunner, "当前 Runner 尚不支持"),
            (PlannerErrorCode::RunnerUnavailable, "系统 Runner 不可用"),
            (
                PlannerErrorCode::InternalContract,
                "Command Block 内部契约无效",
            ),
            (PlannerErrorCode::StalePreview, "Preview 已失效，请重新生成"),
        ];
        for (code, expected_message) in cases {
            let error = public_planner_error(PlannerError {
                code,
                parameter_key: Some("text".to_owned()),
                detail_code: Some("privateValueMustNotAppear".to_owned()),
            });
            assert_eq!(error.code, code.as_str());
            assert_eq!(error.message, expected_message);
            assert_eq!(error.parameter_key.as_deref(), Some("text"));
            assert_eq!(
                error.detail_code.as_deref(),
                Some("privateValueMustNotAppear")
            );
            assert!(!error.message.contains("privateValueMustNotAppear"));
        }
    }

    /// 验证无效 UUID 得到稳定校验错误，不触达 Manager 取消入口。
    #[test]
    fn exposes_stable_invalid_execution_id_error() {
        let result = uuid::Uuid::parse_str("not-an-execution-id")
            .map_err(|_| ApiError::simple("VALIDATION_FAILED", "Execution ID 不是有效的 UUID"));

        assert_eq!(
            result.expect_err("无效 UUID 应失败"),
            ApiError::simple("VALIDATION_FAILED", "Execution ID 不是有效的 UUID")
        );
    }

    /// 验证包含本机临时路径的启动错误不会跨越 IPC 信任边界。
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

    /// 验证 stale、revision 与 validation 在临时目录、Active、线程和事件副作用前失败。
    #[test]
    fn rejects_invalid_runs_before_any_execution_side_effect() {
        let planner = ExecutionPlanner::new();
        let (_, valid_request) = preview_and_run_request(&planner, true);
        let mut stale = valid_request.clone();
        stale.parameter_values.insert(
            "text".to_owned(),
            ParameterValue::Text("Preview 后改变".to_owned()),
        );
        let mut revision = valid_request.clone();
        revision.expected_revision = 2;
        let mut validation = valid_request;
        validation.parameter_values.remove("text");

        for (request, expected_code) in [
            (stale, "STALE_PREVIEW"),
            (revision, "REVISION_CONFLICT"),
            (validation, "VALIDATION_FAILED"),
        ] {
            let manager = ExecutionManager::new();
            let sent = Arc::new(AtomicUsize::new(0));
            let mut observed_quiet_snapshot = false;
            let mut last_new_directories = Vec::new();
            for _ in 0..32 {
                let observed = Arc::clone(&sent);
                let before = temporary_execution_directories();
                let error = run_with_sender(&planner, manager.clone(), &request, move |_| {
                    observed.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
                .expect_err("无效 Run 必须在启动前失败");
                let after = temporary_execution_directories();

                assert_eq!(error.code, expected_code);
                last_new_directories = after.difference(&before).cloned().collect::<Vec<_>>();
                if last_new_directories.is_empty() {
                    observed_quiet_snapshot = true;
                    break;
                }
                // 其他并行 Rust 测试会短暂创建自己的 Artifact；换一个安静窗口重复直属集合比较。
                std::thread::yield_now();
            }
            assert!(
                observed_quiet_snapshot,
                "失败请求不得创建临时 Execution 目录：{last_new_directories:?}"
            );
            assert!(manager.active_snapshot().is_empty());
            assert_eq!(sent.load(Ordering::SeqCst), 0);
        }
    }

    /// 验证 UI Channel 断开只停止转发，不取消底层无害短等待 Session。
    #[test]
    fn channel_failure_does_not_cancel_execution() {
        let manager = ExecutionManager::new();
        let verified = verified_windows_powershell_for_test(
            "[Console]::Out.WriteLine('started'); Start-Sleep -Milliseconds 700; exit 0",
            std::env::temp_dir(),
        );
        let started = manager.start(verified).expect("短等待任务应启动");
        let execution_id = started.execution_id;
        let sent_events = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&sent_events);
        std::thread::spawn(move || {
            super::forward_events(started.events, move |_| {
                observed.fetch_add(1, Ordering::SeqCst);
                Err(())
            });
        });

        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            manager.active_snapshot().len(),
            1,
            "Channel 失败不应取消短等待任务"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !manager.active_snapshot().is_empty() {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(manager.active_snapshot().is_empty(), "任务应自然结束并清理");
        assert_eq!(sent_events.load(Ordering::SeqCst), 1);
        assert!(execution_id.to_string().parse::<uuid::Uuid>().is_ok());
    }

    /// 验证 PowerShell Built-in 经 Preview、复验、Session 和 IPC Adapter 字面回显六类参数。
    #[test]
    fn runs_typed_powershell_built_in_through_ipc_adapter() {
        let temporary_root = std::env::temp_dir();
        fs::create_dir_all(temporary_root.join("CmdBox")).expect("测试前应确保 CmdBox 临时根存在");
        let current_root = std::env::current_dir().expect("测试当前目录应存在");
        let temporary_before = direct_directory_entries(&temporary_root);
        let current_before = direct_directory_entries(&current_root);
        let planner = ExecutionPlanner::new();

        for enabled in [true, false] {
            let (preview, run_request) = preview_and_run_request(&planner, enabled);
            let manager = ExecutionManager::new();
            let channel_events = Arc::new(Mutex::new(Vec::<ExecutionStreamEvent>::new()));
            let observed = Arc::clone(&channel_events);
            let run_response =
                run_with_sender(&planner, manager.clone(), &run_request, move |event| {
                    observed.lock().expect("事件锁不应中毒").push(event);
                    Ok(())
                })
                .expect("当前 Preview 应启动 PowerShell Built-in");
            let run_response = serde_json::to_value(run_response).expect("Run 响应应可序列化");
            assert_eq!(
                run_response
                    .as_object()
                    .expect("run 响应应为对象")
                    .keys()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from(["executionId"])
            );

            let events = wait_for_terminal(&channel_events);
            assert!(matches!(
                events.first(),
                Some(ExecutionStreamEvent::Started { .. })
            ));
            let response_execution_id = run_response["executionId"]
                .as_str()
                .expect("Run 响应应返回 Execution ID");
            assert!(events
                .iter()
                .all(|event| ipc_execution_id(event) == response_execution_id));
            assert_eq!(events.first().map(ipc_sequence), Some(0));
            assert!(events
                .windows(2)
                .all(|pair| ipc_sequence(&pair[1]) == ipc_sequence(&pair[0]).saturating_add(1)));
            assert_eq!(
                events.iter().filter(|event| is_ipc_terminal(event)).count(),
                1
            );
            assert!(matches!(
                events.last(),
                Some(ExecutionStreamEvent::Finished { exit_code: 0, .. })
            ));
            std::thread::sleep(Duration::from_millis(20));
            assert_eq!(
                channel_events.lock().expect("事件锁不应中毒").len(),
                events.len(),
                "终态后不得继续发送事件"
            );
            let stdout = events
                .iter()
                .filter_map(|event| match event {
                    ExecutionStreamEvent::Output { fragments, .. } => Some(fragments),
                    _ => None,
                })
                .flatten()
                .filter(|fragment| fragment.stream == IpcOutputStream::Stdout)
                .map(|fragment| fragment.text.as_str())
                .collect::<String>();
            let summary_values = |key: &str| {
                preview
                    .parameter_summaries
                    .iter()
                    .find(|summary| summary.parameter_key == key)
                    .expect("Preview 应含完整参数摘要")
                    .display_values
                    .clone()
            };
            let mut expected_lines = vec!["中文 空格 user's value".to_owned(), "4".to_owned()];
            if enabled {
                expected_lines.push("enabled".to_owned());
            }
            expected_lines.push("detailed".to_owned());
            expected_lines.extend(summary_values("folder"));
            expected_lines.extend(summary_values("folders"));
            let actual_lines = stdout.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
            assert_eq!(actual_lines, expected_lines, "实际 stdout：{stdout:?}");

            let removal_deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < removal_deadline && !manager.active_snapshot().is_empty() {
                std::thread::yield_now();
            }
            assert!(manager.active_snapshot().is_empty());
        }

        assert_eq!(temporary_before, direct_directory_entries(&temporary_root));
        assert_eq!(current_before, direct_directory_entries(&current_root));
    }

    /// 验证 CMD Built-in 经相同 Preview、复验、Session 和 IPC 接口安全回显全部边界字符。
    #[test]
    fn runs_typed_cmd_built_in_through_ipc_adapter() {
        let target_root = std::env::temp_dir()
            .join("CmdBox")
            .join(format!("test-target-{}", uuid::Uuid::new_v4()));
        let first_target = target_root.join("first target");
        let second_target = target_root.join("second 日本語 target");
        fs::create_dir_all(&first_target).expect("应创建第一测试目标目录");
        fs::create_dir(&second_target).expect("应创建第二测试目标目录");
        let planner = ExecutionPlanner::new();
        let text = "中文 日本語 😀 space ' \" & echo(EXTRA % ^ ! ( ) < > | \\\\ tail";

        for enabled in [true, false] {
            let mut values = valid_values_with_text(enabled, text);
            values.insert(
                "folder".to_owned(),
                ParameterValue::Text(first_target.to_string_lossy().into_owned()),
            );
            values.insert(
                "folders".to_owned(),
                ParameterValue::Array(vec![
                    ParameterValue::Text(first_target.to_string_lossy().into_owned()),
                    ParameterValue::Text(second_target.to_string_lossy().into_owned()),
                ]),
            );
            let preview = planner
                .preview(&PreviewCommandRequest {
                    command_block_id: CMD_PARAMETER_ECHO_ID.to_owned(),
                    expected_revision: 1,
                    parameter_values: values.clone(),
                })
                .expect("CMD Preview 应成功");
            let run_request = VerifyRunRequest {
                command_block_id: CMD_PARAMETER_ECHO_ID.to_owned(),
                expected_revision: preview.revision,
                parameter_values: values,
                execution_spec_hash: preview.execution_spec_hash.clone(),
            };
            assert!(!preview.preview_text.contains(text));
            let manager = ExecutionManager::new();
            let channel_events = Arc::new(Mutex::new(Vec::<ExecutionStreamEvent>::new()));
            let observed = Arc::clone(&channel_events);
            run_with_sender(&planner, manager.clone(), &run_request, move |event| {
                observed.lock().expect("事件锁不应中毒").push(event);
                Ok(())
            })
            .expect("当前 Preview 应启动 CMD Built-in");

            let events = wait_for_terminal(&channel_events);
            assert!(matches!(
                events.first(),
                Some(ExecutionStreamEvent::Started { .. })
            ));
            assert!(matches!(
                events.last(),
                Some(ExecutionStreamEvent::Finished { exit_code: 0, .. })
            ));
            assert_eq!(
                events.iter().filter(|event| is_ipc_terminal(event)).count(),
                1
            );
            let collect_stream = |stream| {
                events
                    .iter()
                    .filter_map(|event| match event {
                        ExecutionStreamEvent::Output { fragments, .. } => Some(fragments),
                        _ => None,
                    })
                    .flatten()
                    .filter(|fragment| fragment.stream == stream)
                    .map(|fragment| fragment.text.as_str())
                    .collect::<String>()
            };
            let stdout = collect_stream(IpcOutputStream::Stdout);
            let stderr = collect_stream(IpcOutputStream::Stderr);
            let summary_values = |key: &str| {
                preview
                    .parameter_summaries
                    .iter()
                    .find(|summary| summary.parameter_key == key)
                    .expect("Preview 应含完整参数摘要")
                    .display_values
                    .clone()
            };
            let mut expected_lines = vec![text.to_owned(), "4".to_owned()];
            if enabled {
                expected_lines.push("enabled".to_owned());
            }
            expected_lines.push("detailed".to_owned());
            expected_lines.extend(summary_values("folder"));
            expected_lines.extend(summary_values("folders"));

            assert_eq!(
                stdout.lines().map(ToOwned::to_owned).collect::<Vec<_>>(),
                expected_lines,
                "注入样式文本不得产生额外 stdout：{stdout:?}"
            );
            assert!(stderr.is_empty(), "注入样式文本不得产生 stderr：{stderr:?}");
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline && !manager.active_snapshot().is_empty() {
                std::thread::yield_now();
            }
            assert!(manager.active_snapshot().is_empty(), "CMD 不得遗留受管进程");
        }

        assert!(direct_directory_entries(&first_target).is_empty());
        assert!(direct_directory_entries(&second_target).is_empty());
        fs::remove_dir(&first_target).expect("第一测试目标应保持为空并可清理");
        fs::remove_dir(&second_target).expect("第二测试目标应保持为空并可清理");
        fs::remove_dir(&target_root).expect("测试目标根应保持为空并可清理");
        assert!(!target_root.exists(), "测试结束不得遗留目标目录");
    }
}
