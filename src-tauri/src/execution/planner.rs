//! Command Block Definition、可信 Preview 与 Run 复验的唯一业务入口。
//!
//! `ExecutionPlanner` 每次都从当前 Rust Built-in 集合重新读取 Definition，再通过同一条
//! Validation → Template AST → Runner Serializer → Canonical Execution Spec 路径工作。
//! 本模块只做计算和必要的系统 Runner 解析，不创建临时脚本或进程；只有 Hash 与 revision
//! 同时匹配时才产出字段私有、调用方无法自行构造的 `VerifiedExecution`。

use std::collections::BTreeMap;
use std::error::Error;
use std::ffi::OsString;
use std::fmt::{Display, Formatter};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::artifact::{ArtifactError, MaterializedScript, RenderedScript};
use super::command::{
    builtin_command_definitions, CommandBlockDefinition, RiskLevel, RunnerType, SafetyPolicy,
};
use super::delete_executor::DeleteExecutionPlan;
use super::parameter::{
    validate_parameter_values, NormalizedParameterValue, NormalizedParameters,
    ParameterValidationError, ParameterValues,
};
use super::safety::{
    inspect_delete_targets, DeleteRiskDecision, DeleteSafetyErrorCode, DeleteSafetyReport,
    PathFingerprint, ProtectedPathSet,
};
use super::serializer::{
    render_cmd, render_windows_powershell, CmdRenderError, PowerShellRenderError,
};
use super::spec::{path_fingerprints_hash_hex, CanonicalExecutionSpec};
use super::template::{parse_template, TemplateError};
use crate::process::windows::runner::{
    CmdRunner, ProcessLaunch, ResolvedRunner, WindowsPowerShellRunner,
};

/// 当前 Canonical Execution Spec 二进制编码版本。
const EXECUTION_SPEC_SCHEMA_VERSION: u32 = 3;

/// 单个参数摘要最多返回的可读值数量。
const PARAMETER_SUMMARY_MAX_VALUES: usize = 5;

/// 参数摘要中的单个可读值最多保留的 Unicode 字符数量。
const PARAMETER_SUMMARY_MAX_CHARS: usize = 160;

/// Preview 脚本文本最多返回给 WebView 的 UTF-8 字节数。
const PREVIEW_TEXT_MAX_BYTES: usize = 32 * 1024;

/// Preview 请求，只允许提交业务身份、期望 revision 和结构化参数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewCommandRequest {
    /// 用户当前打开的 Command Block ID。
    pub command_block_id: String,
    /// 用户基于的 Definition revision。
    #[cfg_attr(test, ts(type = "number"))]
    pub expected_revision: u64,
    /// 尚未信任、必须由 Rust Core 验证的结构化参数值。
    pub parameter_values: ParameterValues,
}

/// Run 复验请求，只增加用户确认的完整 Execution Spec Hash。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerifyRunRequest {
    /// 用户要执行的 Command Block ID。
    pub command_block_id: String,
    /// 用户 Preview 时确认的 Definition revision。
    #[cfg_attr(test, ts(type = "number"))]
    pub expected_revision: u64,
    /// 必须重新验证、规范化和渲染的原始结构化参数值。
    pub parameter_values: ParameterValues,
    /// Preview 返回且用户已经确认的完整 Execution Spec SHA-256。
    pub execution_spec_hash: String,
    /// destructive high-risk Preview 要求的明确确认响应；normal Command 省略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub safety_confirmation: Option<SafetyConfirmationResponse>,
    /// Preview 返回的目标身份凭据；用于在完整 Hash 比较前识别目标被替换。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub target_identity_hash: Option<String>,
}

/// high-risk destructive Run 的窄确认响应。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SafetyConfirmationResponse {
    /// 当前版本固定要求的确认短语。
    pub phrase: String,
}

/// Command Workspace 列表使用的公开 Command Block 字段白名单。
///
/// DTO 故意不含模板、可执行文件、Runner options、工作目录或环境变量，避免列表 IPC 将
/// Rust Core 内部执行定义泄露给 WebView。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase")]
pub struct CommandBlockSummary {
    /// Command Block 的稳定身份。
    pub id: String,
    /// Command Workspace 展示的名称。
    pub name: String,
    /// Command Workspace 展示的用途说明。
    pub description: String,
    /// Built-in 或 User 来源身份。
    pub origin: super::command::CommandOrigin,
    /// Definition 声明的稳定 Runner 类型。
    pub runner: RunnerType,
    /// normal 或 destructive 风险语义。
    pub risk_level: RiskLevel,
    /// 当前 Definition revision。
    #[cfg_attr(test, ts(type = "number"))]
    pub revision: u64,
}

impl From<&CommandBlockDefinition> for CommandBlockSummary {
    /// 从内部完整 Definition 复制公开列表白名单字段。
    fn from(definition: &CommandBlockDefinition) -> Self {
        Self {
            id: definition.id.clone(),
            name: definition.name.clone(),
            description: definition.description.clone(),
            origin: definition.origin,
            runner: definition.runner,
            risk_level: definition.risk_level,
            revision: definition.revision,
        }
    }
}

/// Command Workspace 参数表单使用的公开 Command Block 详情字段白名单。
///
/// Details 在 Summary 字段之外只增加 Parameter Definition；固定模板继续保留在 Rust Core，
/// 前端不能读取、修改或回传模板以绕过 Planner。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase")]
pub struct CommandBlockDetails {
    /// Command Block 的稳定身份。
    pub id: String,
    /// Command Workspace 展示的名称。
    pub name: String,
    /// Command Workspace 展示的用途说明。
    pub description: String,
    /// Built-in 或 User 来源身份。
    pub origin: super::command::CommandOrigin,
    /// Definition 声明的稳定 Runner 类型。
    pub runner: RunnerType,
    /// normal 或 destructive 风险语义。
    pub risk_level: RiskLevel,
    /// 当前 Definition revision。
    #[cfg_attr(test, ts(type = "number"))]
    pub revision: u64,
    /// 按统一 Command Workspace 顺序返回的类型化参数定义。
    pub parameters: Vec<super::parameter::ParameterDefinition>,
}

impl From<&CommandBlockDefinition> for CommandBlockDetails {
    /// 从内部完整 Definition 复制详情白名单字段和类型化参数。
    fn from(definition: &CommandBlockDefinition) -> Self {
        Self {
            id: definition.id.clone(),
            name: definition.name.clone(),
            description: definition.description.clone(),
            origin: definition.origin,
            runner: definition.runner,
            risk_level: definition.risk_level,
            revision: definition.revision,
            parameters: definition.parameters.clone(),
        }
    }
}

/// Rust Core 为一个 Parameter Definition 生成的有界规范化摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase")]
pub struct PreviewParameterSummary {
    /// 对应 Parameter Definition 的稳定 key。
    pub parameter_key: String,
    /// 直接来自当前 Definition 的用户可读名称。
    pub label: String,
    /// 已规范化值的有界可读文本，顺序与 Rust 规范化结果一致。
    pub display_values: Vec<String>,
    /// 未受展示截断影响的完整值数量；标量存在时为一。
    #[cfg_attr(test, ts(type = "number"))]
    pub total_count: u64,
    /// 值数量或任一显示值是否因上限而截断。
    pub truncated: bool,
}

/// Safety Decision 当前可序列化的稳定状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase")]
pub enum PreviewSafetyState {
    /// 当前 normal Built-in 不适用路径安全策略。
    NotApplicable,
    /// 后续安全策略检查全部通过。
    Passed,
    /// 后续安全策略允许执行但需要明确警告。
    Warning,
    /// 后续安全策略阻止执行。
    Blocked,
}

/// 一条由 Rust Safety Guard 生成的稳定警告；当前 normal Built-in 不返回任何项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase")]
pub struct PreviewSafetyWarning {
    /// 前端可以稳定匹配的警告码。
    pub code: String,
    /// 面向用户的警告文本。
    pub message: String,
}

/// Rust Core 对当前完整规范作出的结构化安全结论。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase")]
pub struct PreviewSafetyDecision {
    /// 当前 Safety Policy 的结构化结果。
    pub state: PreviewSafetyState,
    /// 可选的用户可读结论摘要。
    pub summary: Option<String>,
    /// 有顺序的具体警告；当前 normal Built-in 始终为空。
    pub warnings: Vec<PreviewSafetyWarning>,
}

/// 用户可以检查且后续 Run 必须完整复验的 Command Preview。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase")]
pub struct PreviewCommandResponse {
    /// 当前 Preview 对应的 Command Block ID。
    pub command_block_id: String,
    /// 当前 Preview 对应的 Definition revision。
    #[cfg_attr(test, ts(type = "number"))]
    pub revision: u64,
    /// 由当前 Definition 声明且由 Rust 解析的 Runner 类型。
    pub runner: RunnerType,
    /// Rust 规范化参数的有界可读摘要。
    pub parameter_summaries: Vec<PreviewParameterSummary>,
    /// 不含编码前导的有界可读 Runner 脚本文本。
    pub preview_text: String,
    /// 完整最终 Artifact 字节数；编码和 BOM 由当前 Runner 契约决定。
    #[cfg_attr(test, ts(type = "number"))]
    pub full_size_bytes: u64,
    /// `preview_text` 是否因展示上限而被截断。
    pub truncated: bool,
    /// 当前 Definition 的稳定风险语义。
    pub risk_level: RiskLevel,
    /// 当前 normal Built-in 的明确用户动作文案。
    pub action_label: String,
    /// Rust Core 生成的结构化安全结论。
    pub safety: PreviewSafetyDecision,
    /// high-risk Preview 需要的确认要求；普通和 normal Command 为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub confirmation_requirement: Option<PreviewConfirmationRequirement>,
    /// destructive 目标身份列表的稳定凭据；normal Command 为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub target_identity_hash: Option<String>,
    /// 覆盖完整 Canonical Execution Spec 的 64 字符 SHA-256。
    pub execution_spec_hash: String,
}

/// Rust Core 对 high-risk destructive 操作给出的版本化确认要求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase")]
pub struct PreviewConfirmationRequirement {
    /// 进入 Execution Spec 的确认语义版本。
    pub version: u32,
    /// 用户必须精确提交的固定短语。
    pub phrase: String,
}

/// Planner 可跨后续 IPC 稳定映射的错误码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlannerErrorCode {
    /// 当前 Rust Definition 集合没有指定 Command Block。
    CommandBlockNotFound,
    /// 请求 revision 与当前 Definition revision 不一致。
    RevisionConflict,
    /// 一个或多个结构化参数未通过当前 Definition 校验。
    ValidationFailed,
    /// 当前固定 Definition 的受限模板无效。
    InvalidTemplate,
    /// 当前原子尚不支持声明的 Runner。
    UnsupportedRunner,
    /// 系统确定 Runner 无法解析或不可用。
    RunnerUnavailable,
    /// 已校验 AST 与规范化参数之间出现内部契约错误。
    InternalContract,
    /// Run 重建的完整 Execution Spec Hash 与 Preview 不同。
    StalePreview,
    /// destructive 目标根或关键路径被 Safety Guard 阻断。
    SafetyBlocked,
    /// Preview 后目标对象身份发生变化。
    TargetChanged,
    /// high-risk Run 缺少或提交了错误的强化确认。
    ConfirmationRequired,
}

impl PlannerErrorCode {
    /// 返回供日志和后续 IPC 适配稳定使用的 SCREAMING_SNAKE_CASE 标识。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CommandBlockNotFound => "COMMAND_BLOCK_NOT_FOUND",
            Self::RevisionConflict => "REVISION_CONFLICT",
            Self::ValidationFailed => "VALIDATION_FAILED",
            Self::InvalidTemplate => "INVALID_TEMPLATE",
            Self::UnsupportedRunner => "UNSUPPORTED_RUNNER",
            Self::RunnerUnavailable => "RUNNER_UNAVAILABLE",
            Self::InternalContract => "INTERNAL_CONTRACT",
            Self::StalePreview => "STALE_PREVIEW",
            Self::SafetyBlocked => "SAFETY_BLOCKED",
            Self::TargetChanged => "TARGET_CHANGED",
            Self::ConfirmationRequired => "CONFIRMATION_REQUIRED",
        }
    }
}

/// 不包含参数原值、模板正文或本机内部路径的稳定 Planner 错误。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannerError {
    /// 前端和测试可以稳定匹配的顶层错误码。
    pub code: PlannerErrorCode,
    /// 参数或模板错误直接关联的 Parameter key。
    pub parameter_key: Option<String>,
    /// Parameter、Template 或 Renderer 提供的窄原因码。
    pub detail_code: Option<String>,
}

impl PlannerError {
    /// 创建一个不携带本机路径或原始业务值的 Planner 错误。
    fn new(
        code: PlannerErrorCode,
        parameter_key: Option<String>,
        detail_code: Option<String>,
    ) -> Self {
        Self {
            code,
            parameter_key,
            detail_code,
        }
    }
}

/// 输出只包含稳定错误码和可选参数 key 的安全说明。
impl Display for PlannerError {
    /// 格式化不回显原始业务值的 Planner 错误。
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match (&self.parameter_key, &self.detail_code) {
            (Some(key), Some(detail)) => {
                write!(formatter, "{}：{key}（{detail}）", self.code.as_str())
            }
            (Some(key), None) => write!(formatter, "{}：{key}", self.code.as_str()),
            (None, Some(detail)) => write!(formatter, "{}：{detail}", self.code.as_str()),
            (None, None) => formatter.write_str(self.code.as_str()),
        }
    }
}

/// Planner 错误故意不暴露可能包含本机路径的底层错误来源。
impl Error for PlannerError {}

/// Hash 和 revision 已通过 Run 时全量重建复验的执行授权值。
///
/// 所有字段均保持私有；外部调用方只能由 `ExecutionPlanner::verify_run` 获得该值，不能把
/// 请求 JSON、Preview 文本或任意脚本直接构造成已验证执行。
pub struct VerifiedExecution {
    /// 对完整最终字节冻结的 Runner Script Artifact。
    rendered_script: RenderedScript,
    /// 与 Hash 中可执行文件和固定 options 相同的确定 Runner。
    resolved_runner: ResolvedRunner,
    /// 与 Hash 绑定的确定工作目录。
    working_directory: PathBuf,
    /// CMD 完全替换环境中已进入 Hash 的 Definition 与参数绑定；PowerShell 为空。
    environment: BTreeMap<String, OsString>,
    /// 与 Hash 中 version 相同、已通过 Definition 校验的结果解释策略。
    outcome_policy: super::outcome::OutcomePolicy,
    /// normal 与 destructive 的互斥授权种类，避免 Option/布尔组合产生非法状态。
    kind: VerifiedExecutionKind,
}

/// Hash 复验后的唯一执行种类。
enum VerifiedExecutionKind {
    /// 现有普通命令可直接进入标准 Session。
    Standard,
    /// 永久删除只能由可信 Delete Executor 消费有序身份与协议版本。
    Delete {
        fingerprints: Vec<PathFingerprint>,
        collector_protocol_version: u32,
    },
}

/// 只输出授权值的非敏感结构摘要，不输出 Runner、工作目录或参数原值。
impl std::fmt::Debug for VerifiedExecution {
    /// 格式化不包含本机路径和参数原值的授权摘要。
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let (kind, target_count, collector_protocol_version) = match &self.kind {
            VerifiedExecutionKind::Standard => ("standard", 0, None),
            VerifiedExecutionKind::Delete {
                fingerprints,
                collector_protocol_version,
            } => (
                "delete",
                fingerprints.len(),
                Some(*collector_protocol_version),
            ),
        };
        formatter
            .debug_struct("VerifiedExecution")
            .field("artifact_size", &self.rendered_script.bytes().len())
            .field("runner_type", &self.resolved_runner.runner_type())
            .field(
                "working_directory_bound",
                &self.working_directory.is_absolute(),
            )
            .field("kind", &kind)
            .field("target_count", &target_count)
            .field("collector_protocol_version", &collector_protocol_version)
            .finish()
    }
}

impl VerifiedExecution {
    /// 在 Session 启动边界消费唯一授权值，并拆出字段私有的 Launch 与已验证 Policy。
    ///
    /// 只有 `verify_run` 成功后才能调用；本方法会创建受管临时脚本目录并写入已经绑定 Hash
    /// 的完整 Artifact。失败时返回 Artifact 错误且不会启动进程，成功后调用方仍无法修改
    /// executable、Runner options、工作目录、脚本路径或 Policy。
    pub(crate) fn into_session_parts(
        self,
    ) -> Result<(ProcessLaunch, super::outcome::OutcomePolicy), ArtifactError> {
        let Self {
            rendered_script,
            resolved_runner,
            working_directory,
            environment,
            outcome_policy,
            ..
        } = self;
        let materialized_script = MaterializedScript::create(rendered_script)?;
        Ok((
            resolved_runner.process_launch_with_environment(
                materialized_script,
                &working_directory,
                environment,
            ),
            outcome_policy,
        ))
    }

    /// 将 destructive 授权值一次性移交给 Delete Executor 深模块。
    ///
    /// 普通命令返回原授权值，调用方不能把它误当成删除计划；删除命令的脚本、Runner、
    /// 工作目录、环境、Outcome Policy、目标身份和 collector 协议版本则在此被整体消费，
    /// 因而不存在由 Session 或 IPC 重新拼装其中任一字段的旁路。
    #[allow(dead_code)] // CMD04-SESSION-01 将成为生产调用方；本原子先封闭 Executor 唯一接缝。
    pub(crate) fn into_delete_execution_plan(self) -> Option<DeleteExecutionPlan> {
        let VerifiedExecutionKind::Delete {
            fingerprints,
            collector_protocol_version,
        } = &self.kind
        else {
            return None;
        };
        let fingerprints = fingerprints.clone();
        let collector_protocol_version = *collector_protocol_version;
        let Self {
            rendered_script,
            resolved_runner,
            working_directory,
            environment,
            outcome_policy,
            kind: _,
        } = self;
        Some(DeleteExecutionPlan::new(
            rendered_script,
            resolved_runner,
            working_directory,
            environment,
            outcome_policy,
            fingerprints,
            collector_protocol_version,
        ))
    }

    /// 当前授权值是否已有可信 Executor 可安全启动。
    pub(crate) const fn launch_ready(&self) -> bool {
        matches!(self.kind, VerifiedExecutionKind::Standard)
    }
}

/// 为 Session 的底层进程生命周期回归测试构造测试专用授权值。
///
/// 该入口只在测试构建存在，不能成为生产或 IPC 绕过 Preview 的启动旁路。脚本文本只允许由
/// 当前仓库内的无害回显、有限输出、退出码和短等待测试提供。
#[cfg(test)]
pub(crate) fn verified_windows_powershell_for_test(
    script: &str,
    working_directory: PathBuf,
) -> VerifiedExecution {
    verified_windows_powershell_with_policy_for_test(
        script,
        working_directory,
        super::outcome::OutcomePolicy::standard(),
    )
}

/// 为 Session 的特殊结果策略测试构造可注入已校验 Policy 的授权值。
#[cfg(test)]
pub(crate) fn verified_windows_powershell_with_policy_for_test(
    script: &str,
    working_directory: PathBuf,
    outcome_policy: super::outcome::OutcomePolicy,
) -> VerifiedExecution {
    outcome_policy
        .validate()
        .expect("测试专用 Outcome Policy 必须有效");
    VerifiedExecution {
        rendered_script: RenderedScript::windows_powershell(script),
        resolved_runner: WindowsPowerShellRunner::resolve()
            .expect("测试系统应提供 Windows PowerShell"),
        working_directory,
        environment: BTreeMap::new(),
        outcome_policy,
        kind: VerifiedExecutionKind::Standard,
    }
}

/// 两个 Serializer 投影到 Planner 所需的共同冻结结果。
struct PreparedRendered {
    /// Preview 展示的最终脚本文本。
    script_text: String,
    /// Preview 与 Run 共用的最终 Artifact 字节。
    artifact: RenderedScript,
    /// CMD 非空参数使用的确定性私有环境绑定；PowerShell 为空。
    private_environment: BTreeMap<String, OsString>,
}

/// Safety Policy 应用后的报告、确认语义版本和 collector 协议版本。
type AppliedSafetyPolicy = (Option<DeleteSafetyReport>, Option<u32>, Option<u32>);

/// Preview 与 Run 复验共用的私有完整计算结果。
struct PreparedExecution {
    /// 当前重新读取的完整 Command Block Definition。
    definition: CommandBlockDefinition,
    /// 当前请求重新校验后的规范化参数。
    normalized_parameters: NormalizedParameters,
    /// 当前系统重新解析得到的确定 Runner。
    resolved_runner: ResolvedRunner,
    /// 当前 AST 重新渲染得到的可读文本和完整 Artifact。
    rendered: PreparedRendered,
    /// 当前执行使用并进入 Hash 的确定工作目录。
    working_directory: PathBuf,
    /// 启动时使用且已进入 Hash 的 Definition 与私有参数环境。
    environment: BTreeMap<String, OsString>,
    /// 当前全部 Execution Spec 事实计算得到的 SHA-256。
    execution_spec_hash: String,
    /// 当前 Safety Guard 的结构化输出；normal Command 为空。
    safety_report: Option<DeleteSafetyReport>,
    /// high-risk 确认语义版本。
    confirmation_requirement_version: Option<u32>,
    /// 已进入 Hash、必须交给固定 Executor 的 collector 逻辑协议版本。
    collector_protocol_version: Option<u32>,
}

/// 固定 Built-in Definition 的可信 Preview 与 Run 复验深模块。
#[derive(Debug, Clone, Copy, Default)]
pub struct ExecutionPlanner;

impl ExecutionPlanner {
    /// 创建一个不持有 Preview 缓存或可变执行状态的 Planner。
    pub const fn new() -> Self {
        Self
    }

    /// 按稳定 Built-in 顺序返回不含模板和启动配置的 Command Block Summary。
    pub fn list_command_blocks(&self) -> Vec<CommandBlockSummary> {
        builtin_command_definitions()
            .iter()
            .map(CommandBlockSummary::from)
            .collect()
    }

    /// 从当前 Built-in 集合读取不含模板和启动配置的 Command Block Details。
    pub fn get_command_block(
        &self,
        command_block_id: &str,
    ) -> Result<CommandBlockDetails, PlannerError> {
        load_current_definition(command_block_id).map(|definition| (&definition).into())
    }

    /// 校验当前 revision 和结构化值，并生成绑定完整 PowerShell Execution Spec 的 Preview。
    pub fn preview(
        &self,
        request: &PreviewCommandRequest,
    ) -> Result<PreviewCommandResponse, PlannerError> {
        let definition = load_current_definition(&request.command_block_id)?;
        ensure_revision(&definition, request.expected_revision)?;
        let prepared = prepare_execution(definition, &request.parameter_values)?;
        build_preview_response(prepared, PREVIEW_TEXT_MAX_BYTES)
    }

    /// 全量重建当前 Execution Spec，先检查 revision，再比较 Hash 并产出不可构造授权值。
    pub fn verify_run(
        &self,
        request: &VerifyRunRequest,
    ) -> Result<VerifiedExecution, PlannerError> {
        let definition = load_current_definition(&request.command_block_id)?;
        ensure_revision(&definition, request.expected_revision)?;
        let prepared = prepare_execution(definition, &request.parameter_values)?;
        if let Some(report) = prepared.safety_report.as_ref() {
            let current = path_fingerprints_hash_hex(
                &report
                    .targets
                    .iter()
                    .map(|target| target.fingerprint.clone())
                    .collect::<Vec<_>>(),
            );
            if request.target_identity_hash.as_deref() != Some(current.as_str()) {
                return Err(PlannerError::new(
                    PlannerErrorCode::TargetChanged,
                    None,
                    None,
                ));
            }
        }
        validate_safety_confirmation(
            prepared.confirmation_requirement_version,
            request.safety_confirmation.as_ref(),
        )?;
        if prepared.execution_spec_hash != request.execution_spec_hash {
            return Err(PlannerError::new(
                PlannerErrorCode::StalePreview,
                None,
                None,
            ));
        }

        let kind = match prepared.safety_report {
            None => VerifiedExecutionKind::Standard,
            Some(report) => {
                let Some(collector_protocol_version) = prepared.collector_protocol_version else {
                    return Err(PlannerError::new(
                        PlannerErrorCode::InternalContract,
                        None,
                        Some("deleteCollectorProtocolMissing".to_owned()),
                    ));
                };
                VerifiedExecutionKind::Delete {
                    fingerprints: report
                        .targets
                        .into_iter()
                        .map(|target| target.fingerprint)
                        .collect(),
                    collector_protocol_version,
                }
            }
        };
        Ok(VerifiedExecution {
            rendered_script: prepared.rendered.artifact,
            resolved_runner: prepared.resolved_runner,
            working_directory: prepared.working_directory,
            environment: prepared.environment,
            outcome_policy: prepared.definition.outcome_policy,
            kind,
        })
    }
}

/// 验证版本化 high-risk 确认；当前版本只接受大小写精确的固定短语。
fn validate_safety_confirmation(
    requirement_version: Option<u32>,
    response: Option<&SafetyConfirmationResponse>,
) -> Result<(), PlannerError> {
    match requirement_version {
        None => Ok(()),
        Some(1) if response.is_some_and(|confirmation| confirmation.phrase == "DELETE") => Ok(()),
        Some(1) => Err(PlannerError::new(
            PlannerErrorCode::ConfirmationRequired,
            None,
            None,
        )),
        Some(_) => Err(PlannerError::new(
            PlannerErrorCode::InternalContract,
            None,
            Some("unsupportedConfirmationVersion".to_owned()),
        )),
    }
}

/// 每次调用都从 Rust 固定集合重新读取 Definition，不复用 Preview 计算结果。
fn load_current_definition(command_block_id: &str) -> Result<CommandBlockDefinition, PlannerError> {
    builtin_command_definitions()
        .into_iter()
        .find(|definition| definition.id == command_block_id)
        .ok_or_else(|| PlannerError::new(PlannerErrorCode::CommandBlockNotFound, None, None))
}

/// 在参数、模板或 Runner 计算前拒绝已经过期的 Definition revision。
fn ensure_revision(
    definition: &CommandBlockDefinition,
    expected_revision: u64,
) -> Result<(), PlannerError> {
    if definition.revision != expected_revision {
        return Err(PlannerError::new(
            PlannerErrorCode::RevisionConflict,
            None,
            None,
        ));
    }
    Ok(())
}

/// 执行 Preview 与 Run 共用的唯一纯计算链并冻结完整 Canonical Hash。
fn prepare_execution(
    definition: CommandBlockDefinition,
    parameter_values: &ParameterValues,
) -> Result<PreparedExecution, PlannerError> {
    validate_definition_safety_contract(&definition)?;
    definition.outcome_policy.validate().map_err(|error| {
        let detail_code = match error {
            super::outcome::OutcomePolicyError::ZeroVersion => "outcomePolicyZeroVersion",
            super::outcome::OutcomePolicyError::InvalidRange => "outcomePolicyInvalidRange",
            super::outcome::OutcomePolicyError::OverlappingRanges => {
                "outcomePolicyOverlappingRanges"
            }
        };
        PlannerError::new(
            PlannerErrorCode::InternalContract,
            None,
            Some(detail_code.to_owned()),
        )
    })?;
    let mut normalized_parameters =
        validate_parameter_values(&definition.parameters, parameter_values)
            .map_err(planner_parameter_error)?;
    let (safety_report, confirmation_requirement_version, collector_protocol_version) =
        apply_safety_policy(&definition, &mut normalized_parameters)?;
    let ast = parse_template(&definition.template, &definition.parameters)
        .map_err(planner_template_error)?;
    let (rendered, resolved_runner) = match definition.runner {
        RunnerType::WindowsPowerShell => {
            if !definition.environment.is_empty() {
                return Err(PlannerError::new(
                    PlannerErrorCode::InternalContract,
                    None,
                    Some("powerShellEnvironmentMustInherit".to_owned()),
                ));
            }
            let rendered = render_windows_powershell(&ast, &normalized_parameters)
                .map_err(planner_render_error)?;
            let runner = WindowsPowerShellRunner::resolve()
                .map_err(|_| PlannerError::new(PlannerErrorCode::RunnerUnavailable, None, None))?;
            (
                PreparedRendered {
                    script_text: rendered.script_text,
                    artifact: rendered.artifact,
                    private_environment: BTreeMap::new(),
                },
                runner,
            )
        }
        RunnerType::Cmd => {
            validate_cmd_definition_environment(&definition.environment)?;
            let rendered =
                render_cmd(&ast, &normalized_parameters).map_err(planner_cmd_render_error)?;
            let runner = CmdRunner::resolve()
                .map_err(|_| PlannerError::new(PlannerErrorCode::RunnerUnavailable, None, None))?;
            (
                PreparedRendered {
                    script_text: rendered.script_text,
                    artifact: rendered.artifact,
                    private_environment: rendered.private_environment,
                },
                runner,
            )
        }
    };
    let working_directory = std::env::temp_dir();
    let explicit_environment = definition.environment.clone();
    let mut environment = explicit_environment
        .iter()
        .map(|(name, value)| (name.clone(), OsString::from(value)))
        .collect::<BTreeMap<_, _>>();
    environment.extend(rendered.private_environment.clone());
    let mut internal_environment = resolved_runner.fixed_environment().clone();
    internal_environment.extend(rendered.private_environment.clone());
    let execution_spec_hash = CanonicalExecutionSpec {
        schema_version: EXECUTION_SPEC_SCHEMA_VERSION,
        command_block_id: definition.id.clone(),
        revision: definition.revision,
        runner_type: resolved_runner.runner_type().as_str().to_owned(),
        runner_executable: resolved_runner.executable().to_path_buf(),
        runner_fixed_options: resolved_runner.fixed_arguments().to_vec(),
        runner_raw_command_tail: resolved_runner.raw_command_tail().cloned(),
        artifact_hash: rendered.artifact.artifact_hash(),
        normalized_parameters: normalized_parameters.clone(),
        working_directory: working_directory.clone(),
        explicit_environment: explicit_environment.clone(),
        internal_environment,
        safety_policy_version: definition.safety_policy.version(),
        path_fingerprints: safety_report
            .as_ref()
            .map(|report| {
                report
                    .targets
                    .iter()
                    .map(|target| target.fingerprint.clone())
                    .collect()
            })
            .unwrap_or_default(),
        safety_decision: match safety_report.as_ref().map(|report| report.risk) {
            None => "notApplicable",
            Some(DeleteRiskDecision::Normal) => "passed",
            Some(DeleteRiskDecision::HighRisk) => "warning",
        }
        .to_owned(),
        confirmation_requirement_version,
        collector_protocol_version,
        outcome_policy_version: definition.outcome_policy.version(),
    }
    .hash_hex();

    Ok(PreparedExecution {
        definition,
        normalized_parameters,
        resolved_runner,
        rendered,
        working_directory,
        environment,
        execution_spec_hash,
        safety_report,
        confirmation_requirement_version,
        collector_protocol_version,
    })
}

/// 拒绝风险等级、Safety Policy 或版本配置不一致的可信 Definition。
fn validate_definition_safety_contract(
    definition: &CommandBlockDefinition,
) -> Result<(), PlannerError> {
    let valid = match (&definition.risk_level, &definition.safety_policy) {
        (RiskLevel::Normal, SafetyPolicy::Generic { version }) => *version > 0,
        (
            RiskLevel::Destructive,
            SafetyPolicy::DeletePaths {
                version,
                confirmation_version,
                collector_protocol_version,
                ..
            },
        ) => {
            *version == super::safety::DELETE_PATH_POLICY_VERSION
                && *confirmation_version == 1
                && *collector_protocol_version == 1
        }
        _ => false,
    };
    if !valid {
        return Err(PlannerError::new(
            PlannerErrorCode::InternalContract,
            None,
            Some("invalidSafetyPolicy".to_owned()),
        ));
    }
    Ok(())
}

/// 将 Definition 的结构化 Safety Policy 应用于已类型化参数，并让删除模板只看到折叠目标。
fn apply_safety_policy(
    definition: &CommandBlockDefinition,
    normalized: &mut NormalizedParameters,
) -> Result<AppliedSafetyPolicy, PlannerError> {
    if matches!(definition.safety_policy, SafetyPolicy::Generic { .. }) {
        return Ok((None, None, None));
    }
    let protected = ProtectedPathSet::for_cmdbox().map_err(|_| {
        PlannerError::new(
            PlannerErrorCode::InternalContract,
            None,
            Some("protectedPathsUnavailable".to_owned()),
        )
    })?;
    apply_safety_policy_with_protected(definition, normalized, &protected)
}

/// 使用调用方已经建立的保护根执行删除策略；测试可注入隔离目录验证 high-risk 分支。
fn apply_safety_policy_with_protected(
    definition: &CommandBlockDefinition,
    normalized: &mut NormalizedParameters,
    protected: &ProtectedPathSet,
) -> Result<AppliedSafetyPolicy, PlannerError> {
    let SafetyPolicy::DeletePaths {
        parameter_key,
        confirmation_version,
        collector_protocol_version,
        ..
    } = &definition.safety_policy
    else {
        return Ok((None, None, None));
    };
    let Some(entry) = normalized
        .entries
        .iter_mut()
        .find(|entry| entry.key == *parameter_key)
    else {
        return Err(PlannerError::new(
            PlannerErrorCode::InternalContract,
            Some(parameter_key.clone()),
            Some("deletePathsParameterMissing".to_owned()),
        ));
    };
    let Some(NormalizedParameterValue::Folders(values)) = entry.value.as_mut() else {
        return Err(PlannerError::new(
            PlannerErrorCode::InternalContract,
            Some(parameter_key.clone()),
            Some("deletePathsParameterMustBeFolders".to_owned()),
        ));
    };
    let report = inspect_delete_targets(values, protected).map_err(|error| {
        let code = match error.code {
            DeleteSafetyErrorCode::CriticalPath
            | DeleteSafetyErrorCode::ReparsePoint
            | DeleteSafetyErrorCode::DangerousNamespace => PlannerErrorCode::SafetyBlocked,
            DeleteSafetyErrorCode::TargetChanged => PlannerErrorCode::TargetChanged,
            _ => PlannerErrorCode::ValidationFailed,
        };
        PlannerError::new(
            code,
            Some(parameter_key.clone()),
            Some(error.code.as_str().to_owned()),
        )
    })?;
    *values = report
        .targets
        .iter()
        .map(|target| {
            target
                .fingerprint
                .normalized_path
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| {
                    PlannerError::new(
                        PlannerErrorCode::InternalContract,
                        Some(parameter_key.clone()),
                        Some("deletePathEncodingChanged".to_owned()),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let confirmation =
        (report.risk == DeleteRiskDecision::HighRisk).then_some(*confirmation_version);
    Ok((
        Some(report),
        confirmation,
        Some(*collector_protocol_version),
    ))
}

/// 校验 CMD Definition 环境键，并保护所有 `CMDBOX_INTERNAL_` 私有名称不被配置覆盖。
fn validate_cmd_definition_environment(
    environment: &BTreeMap<String, String>,
) -> Result<(), PlannerError> {
    let mut folded_names = std::collections::BTreeSet::new();
    for (name, value) in environment {
        if name.is_empty()
            || !name.is_ascii()
            || name.contains('=')
            || name.contains('\0')
            || value.contains('\0')
        {
            return Err(PlannerError::new(
                PlannerErrorCode::InternalContract,
                None,
                Some("invalidCmdEnvironment".to_owned()),
            ));
        }
        let folded = name.to_ascii_uppercase();
        if folded.starts_with("CMDBOX_INTERNAL_") || folded == "SYSTEMROOT" {
            return Err(PlannerError::new(
                PlannerErrorCode::InternalContract,
                None,
                Some("reservedCmdEnvironmentName".to_owned()),
            ));
        }
        if !folded_names.insert(folded) {
            return Err(PlannerError::new(
                PlannerErrorCode::InternalContract,
                None,
                Some("duplicateCmdEnvironmentName".to_owned()),
            ));
        }
    }
    Ok(())
}

/// 将完整计算结果投影为有界显示 Response，展示截断不参与 Hash。
fn build_preview_response(
    prepared: PreparedExecution,
    preview_text_max_bytes: usize,
) -> Result<PreviewCommandResponse, PlannerError> {
    let parameter_summaries =
        build_parameter_summaries(&prepared.definition, &prepared.normalized_parameters)?;
    let full_size_bytes = prepared.rendered.artifact.bytes().len() as u64;
    let (preview_text, truncated) =
        truncate_utf8(&prepared.rendered.script_text, preview_text_max_bytes);

    let (action_label, safety, confirmation_requirement, target_identity_hash) =
        if let Some(report) = &prepared.safety_report {
            let mut warnings = Vec::new();
            if report.folded_count > 0 {
                warnings.push(PreviewSafetyWarning {
                    code: "ANCESTOR_COLLAPSED".to_owned(),
                    message: format!(
                        "{} 个重复或子目录已折叠，不会重复删除。",
                        report.folded_count
                    ),
                });
            }
            if report.risk == DeleteRiskDecision::HighRisk {
                warnings.push(PreviewSafetyWarning {
                    code: "HIGH_RISK_USER_ROOT".to_owned(),
                    message: "所选目标包含常用用户目录根，需要输入 DELETE 确认。".to_owned(),
                });
            }
            let fingerprints = report
                .targets
                .iter()
                .map(|target| target.fingerprint.clone())
                .collect::<Vec<_>>();
            (
                "永久删除".to_owned(),
                PreviewSafetyDecision {
                    state: if report.risk == DeleteRiskDecision::HighRisk {
                        PreviewSafetyState::Warning
                    } else {
                        PreviewSafetyState::Passed
                    },
                    summary: Some(format!(
                        "将永久删除 {} 个目录，不经过回收站。",
                        report.targets.len()
                    )),
                    warnings,
                },
                prepared.confirmation_requirement_version.map(|version| {
                    PreviewConfirmationRequirement {
                        version,
                        phrase: "DELETE".to_owned(),
                    }
                }),
                Some(path_fingerprints_hash_hex(&fingerprints)),
            )
        } else {
            (
                "执行".to_owned(),
                PreviewSafetyDecision {
                    state: PreviewSafetyState::NotApplicable,
                    summary: None,
                    warnings: Vec::new(),
                },
                None,
                None,
            )
        };

    Ok(PreviewCommandResponse {
        command_block_id: prepared.definition.id,
        revision: prepared.definition.revision,
        runner: prepared.definition.runner,
        parameter_summaries,
        preview_text,
        full_size_bytes,
        truncated,
        risk_level: prepared.definition.risk_level,
        action_label,
        safety,
        confirmation_requirement,
        target_identity_hash,
        execution_spec_hash: prepared.execution_spec_hash,
    })
}

/// 按 Definition 顺序为每个规范化参数生成有界可读摘要。
fn build_parameter_summaries(
    definition: &CommandBlockDefinition,
    normalized: &NormalizedParameters,
) -> Result<Vec<PreviewParameterSummary>, PlannerError> {
    let mut summaries = Vec::with_capacity(definition.parameters.len());
    for parameter_definition in &definition.parameters {
        let Some(entry) = normalized
            .entries
            .iter()
            .find(|entry| entry.key == parameter_definition.key())
        else {
            return Err(PlannerError::new(
                PlannerErrorCode::InternalContract,
                Some(parameter_definition.key().to_owned()),
                Some("missingNormalizedParameter".to_owned()),
            ));
        };
        let (display_values, total_count, truncated) =
            summarize_parameter_value(entry.value.as_ref());
        summaries.push(PreviewParameterSummary {
            parameter_key: entry.key.clone(),
            label: parameter_definition.base().label.clone(),
            display_values,
            total_count,
            truncated,
        });
    }
    Ok(summaries)
}

/// 将一个规范化值转换为有数量真值的有界显示集合。
fn summarize_parameter_value(value: Option<&NormalizedParameterValue>) -> (Vec<String>, u64, bool) {
    let Some(value) = value else {
        return (Vec::new(), 0, false);
    };

    match value {
        NormalizedParameterValue::Folders(values) => {
            let mut truncated = values.len() > PARAMETER_SUMMARY_MAX_VALUES;
            let display_values = values
                .iter()
                .take(PARAMETER_SUMMARY_MAX_VALUES)
                .map(|value| {
                    let (display, value_truncated) =
                        truncate_chars(value, PARAMETER_SUMMARY_MAX_CHARS);
                    truncated |= value_truncated;
                    display
                })
                .collect();
            (display_values, values.len() as u64, truncated)
        }
        NormalizedParameterValue::Text(value)
        | NormalizedParameterValue::Select(value)
        | NormalizedParameterValue::Folder(value) => {
            let (display, truncated) = truncate_chars(value, PARAMETER_SUMMARY_MAX_CHARS);
            (vec![display], 1, truncated)
        }
        NormalizedParameterValue::Number(value) => {
            let display = if *value == 0.0 {
                "0".to_owned()
            } else {
                value.to_string()
            };
            (vec![display], 1, false)
        }
        NormalizedParameterValue::Boolean(value) => (vec![value.to_string()], 1, false),
    }
}

/// 按 Unicode 字符数截断单个摘要值，并用省略号明确标记展示不完整。
fn truncate_chars(value: &str, max_chars: usize) -> (String, bool) {
    let total_chars = value.chars().count();
    if total_chars <= max_chars {
        return (value.to_owned(), false);
    }
    if max_chars == 0 {
        return (String::new(), true);
    }

    let mut truncated = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    (truncated, true)
}

/// 按 UTF-8 字节上限截断 Preview 文本，并保证结尾位于合法字符边界。
fn truncate_utf8(value: &str, max_bytes: usize) -> (String, bool) {
    if value.len() <= max_bytes {
        return (value.to_owned(), false);
    }

    let mut end = max_bytes.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_owned(), true)
}

/// 将参数校验错误映射为不回显输入值的 Planner 契约。
fn planner_parameter_error(error: ParameterValidationError) -> PlannerError {
    PlannerError::new(
        PlannerErrorCode::ValidationFailed,
        Some(error.key),
        Some(error.code.as_str().to_owned()),
    )
}

/// 将模板错误映射为不回显模板正文的 Planner 契约。
fn planner_template_error(error: TemplateError) -> PlannerError {
    PlannerError::new(
        PlannerErrorCode::InvalidTemplate,
        error.key,
        Some(error.code.as_str().to_owned()),
    )
}

/// 将内部 Renderer 契约错误映射为不包含参数值的稳定 Planner 错误。
fn planner_render_error(error: PowerShellRenderError) -> PlannerError {
    PlannerError::new(
        PlannerErrorCode::InternalContract,
        error.parameter_key,
        Some(error.code.as_str().to_owned()),
    )
}

/// 将 CMD 行级模板或值边界错误映射为不回显原值的稳定 Planner 契约。
fn planner_cmd_render_error(error: CmdRenderError) -> PlannerError {
    let code = if error.code.is_template_error() {
        PlannerErrorCode::InvalidTemplate
    } else if error.code.is_validation_error() {
        PlannerErrorCode::ValidationFailed
    } else {
        PlannerErrorCode::InternalContract
    };
    PlannerError::new(
        code,
        error.parameter_key,
        Some(error.code.as_str().to_owned()),
    )
}

#[cfg(test)]
mod tests {
    //! Planner 的 Definition、Preview、复验、摘要边界和无副作用 Hash 测试。

    use std::collections::BTreeMap;
    #[cfg(feature = "delete-validation")]
    use std::fs;
    #[cfg(feature = "delete-validation")]
    use std::path::{Path, PathBuf};

    #[cfg(feature = "delete-validation")]
    use super::{apply_safety_policy_with_protected, validate_safety_confirmation};
    use super::{
        build_preview_response, prepare_execution, ExecutionPlanner, PlannerErrorCode,
        PreviewCommandRequest, PreviewSafetyState, VerifyRunRequest,
    };
    #[cfg(feature = "delete-validation")]
    use crate::execution::command::DELETE_FOLDERS_ID;
    use crate::execution::command::{
        builtin_command_definitions, CommandBlockDefinition, CommandOrigin, RiskLevel, RunnerType,
        SafetyPolicy, CMD_PARAMETER_ECHO_ID, POWERSHELL_PARAMETER_ECHO_ID,
    };
    #[cfg(feature = "delete-validation")]
    use crate::execution::manager::ExecutionManager;
    use crate::execution::outcome::{ExitCodeRange, OutcomePolicy};
    #[cfg(feature = "delete-validation")]
    use crate::execution::parameter::validate_parameter_values;
    use crate::execution::parameter::{
        ParameterBase, ParameterDefinition, ParameterValue, TextParameterDefinition,
    };
    #[cfg(feature = "delete-validation")]
    use crate::execution::safety::{DeleteRiskDecision, ProtectedPathSet};
    #[cfg(feature = "delete-validation")]
    use crate::execution::session::ExecutionStartError;

    /// 自动清理且只允许位于 `%TEMP%\CmdBox\spec-*` 的测试目录根。
    #[cfg(feature = "delete-validation")]
    struct IsolatedDeleteRoot(PathBuf);

    #[cfg(feature = "delete-validation")]
    impl IsolatedDeleteRoot {
        /// 创建当前测试唯一拥有的空目录根。
        fn create() -> Self {
            let root = std::env::temp_dir()
                .join("CmdBox")
                .join(format!("spec-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&root).expect("应能创建隔离 SPEC 测试根");
            Self(root)
        }

        /// 返回隔离根路径。
        fn path(&self) -> &Path {
            &self.0
        }
    }

    #[cfg(feature = "delete-validation")]
    impl Drop for IsolatedDeleteRoot {
        /// 只清理本测试创建且经过固定父目录和 UUID 前缀校验的根。
        fn drop(&mut self) {
            let expected_parent = std::env::temp_dir().join("CmdBox");
            let valid_name = self
                .0
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("spec-") && name.len() > 20);
            assert_eq!(self.0.parent(), Some(expected_parent.as_path()));
            assert!(valid_name, "拒绝清理不符合 SPEC 测试命名的目录");
            if self.0.exists() {
                fs::remove_dir_all(&self.0).expect("应能清理隔离 SPEC 测试根");
            }
        }
    }

    /// 返回固定 Built-in 参数校验可读取的两个真实目录文本。
    fn existing_folders() -> (String, String) {
        (
            std::env::temp_dir().to_string_lossy().into_owned(),
            std::env::current_dir()
                .expect("测试工作目录应存在")
                .to_string_lossy()
                .into_owned(),
        )
    }

    /// 以刻意非 Definition 顺序创建 PowerShell Built-in 的完整结构化值。
    fn valid_values(enabled: bool) -> BTreeMap<String, ParameterValue> {
        let (first_folder, second_folder) = existing_folders();
        let mut values = BTreeMap::new();
        values.insert(
            "folders".to_owned(),
            ParameterValue::Array(vec![
                ParameterValue::Text(first_folder.clone()),
                ParameterValue::Text(second_folder),
            ]),
        );
        values.insert(
            "mode".to_owned(),
            ParameterValue::Text("detailed".to_owned()),
        );
        values.insert("enabled".to_owned(), ParameterValue::Boolean(enabled));
        values.insert("count".to_owned(), ParameterValue::Number(4.0));
        values.insert(
            "text".to_owned(),
            ParameterValue::Text("中文 空格 user's value".to_owned()),
        );
        values.insert("folder".to_owned(), ParameterValue::Text(first_folder));
        values
    }

    /// 创建固定 PowerShell Built-in 的 Preview 请求。
    fn preview_request(values: BTreeMap<String, ParameterValue>) -> PreviewCommandRequest {
        PreviewCommandRequest {
            command_block_id: POWERSHELL_PARAMETER_ECHO_ID.to_owned(),
            expected_revision: 1,
            parameter_values: values,
        }
    }

    /// 验证 Definition 的 Outcome Policy version 变化会使完整 Execution Spec Hash 变化。
    #[test]
    fn outcome_policy_version_changes_execution_spec_hash() {
        let definition = builtin_command_definitions()
            .into_iter()
            .find(|definition| definition.id == POWERSHELL_PARAMETER_ECHO_ID)
            .expect("PowerShell Built-in 应存在");
        let first = prepare_execution(definition.clone(), &valid_values(true))
            .expect("基线 Definition 应可准备");
        let mut changed = definition;
        changed.outcome_policy =
            OutcomePolicy::exit_code(2, vec![ExitCodeRange { start: 0, end: 0 }], Vec::new());
        let second = prepare_execution(changed, &valid_values(true))
            .expect("只改变合法 Policy version 后仍应可准备");

        assert_ne!(first.execution_spec_hash, second.execution_spec_hash);
    }

    /// 验证非法固定 Policy 在模板渲染前收敛为稳定内部契约错误。
    #[test]
    fn rejects_invalid_outcome_policy_as_internal_contract() {
        let mut definition = builtin_command_definitions()
            .into_iter()
            .find(|definition| definition.id == POWERSHELL_PARAMETER_ECHO_ID)
            .expect("PowerShell Built-in 应存在");
        definition.outcome_policy = OutcomePolicy::target_results(0);

        let error = prepare_execution(definition, &valid_values(true))
            .err()
            .expect("零版本 Policy 应在准备执行时被拒绝");

        assert_eq!(error.code, PlannerErrorCode::InternalContract);
        assert_eq!(
            error.detail_code.as_deref(),
            Some("outcomePolicyZeroVersion")
        );
    }

    /// 验证 destructive 风险不能搭配 Generic 策略，Safety 各版本也不能为零。
    #[test]
    fn rejects_mismatched_or_zero_safety_policy() {
        let mut definition = builtin_command_definitions()[0].clone();
        definition.risk_level = RiskLevel::Destructive;
        let error = prepare_execution(definition, &valid_values(true))
            .err()
            .expect("destructive 风险不得绕过 DeletePaths Safety Policy");
        assert_eq!(error.code, PlannerErrorCode::InternalContract);
        assert_eq!(error.detail_code.as_deref(), Some("invalidSafetyPolicy"));

        let mut definition = builtin_command_definitions()[0].clone();
        definition.safety_policy = SafetyPolicy::Generic { version: 0 };
        let error = prepare_execution(definition, &valid_values(true))
            .err()
            .expect("Safety Policy 零版本必须拒绝");
        assert_eq!(error.detail_code.as_deref(), Some("invalidSafetyPolicy"));
    }

    /// 验证 Planner 列表/详情只返回公开字段白名单，并且不会序列化内部模板。
    #[test]
    fn lists_and_gets_public_command_block_dtos_without_template() {
        let planner = ExecutionPlanner::new();
        let summaries = planner.list_command_blocks();

        let expected_count =
            2 + if cfg!(feature = "ui-validation") {
                3
            } else {
                0
            } + usize::from(cfg!(feature = "delete-validation"));
        assert_eq!(summaries.len(), expected_count);
        assert_eq!(summaries[0].id, POWERSHELL_PARAMETER_ECHO_ID);
        let summary_json = serde_json::to_value(&summaries[0]).expect("Summary 应可序列化");
        assert_eq!(
            summary_json
                .as_object()
                .expect("Summary 应为 JSON Object")
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from([
                "description",
                "id",
                "name",
                "origin",
                "revision",
                "riskLevel",
                "runner",
            ])
        );

        let details = planner
            .get_command_block(POWERSHELL_PARAMETER_ECHO_ID)
            .expect("PowerShell Details 应存在");
        assert_eq!(details.id, summaries[0].id);
        let details_json = serde_json::to_value(&details).expect("Details 应可序列化");
        assert_eq!(
            details_json
                .as_object()
                .expect("Details 应为 JSON Object")
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from([
                "description",
                "id",
                "name",
                "origin",
                "parameters",
                "revision",
                "riskLevel",
                "runner",
            ])
        );
        assert!(details_json.get("template").is_none());

        let missing = planner
            .get_command_block("missing")
            .expect_err("未知 Command Block 应拒绝");
        assert_eq!(missing.code, PlannerErrorCode::CommandBlockNotFound);
    }

    /// 验证请求 DTO 拒绝脚本、可执行文件和其他未声明旁路字段。
    #[test]
    fn rejects_unknown_preview_and_run_request_fields() {
        let preview_error = serde_json::from_value::<PreviewCommandRequest>(serde_json::json!({
            "commandBlockId": POWERSHELL_PARAMETER_ECHO_ID,
            "expectedRevision": 1,
            "parameterValues": {},
            "script": "Write-Output 'bypass'"
        }))
        .expect_err("Preview 不应接受 script 旁路字段");
        assert!(preview_error.to_string().contains("unknown field"));

        let run_error = serde_json::from_value::<VerifyRunRequest>(serde_json::json!({
            "commandBlockId": POWERSHELL_PARAMETER_ECHO_ID,
            "expectedRevision": 1,
            "parameterValues": {},
            "executionSpecHash": "0".repeat(64),
            "executable": "cmd.exe"
        }))
        .expect_err("Run 不应接受 executable 旁路字段");
        assert!(run_error.to_string().contains("unknown field"));
    }

    /// 验证中文、空格、单引号、多路径和 if/each 进入确定的 Rust Preview 与 normal Safety。
    #[test]
    fn previews_normal_powershell_with_normalized_summary_and_full_hash() {
        let planner = ExecutionPlanner::new();
        let response = planner
            .preview(&preview_request(valid_values(true)))
            .expect("PowerShell Built-in 应生成 Preview");

        assert_eq!(response.command_block_id, POWERSHELL_PARAMETER_ECHO_ID);
        assert_eq!(response.runner, RunnerType::WindowsPowerShell);
        assert_eq!(response.risk_level, RiskLevel::Normal);
        assert_eq!(response.action_label, "执行");
        assert_eq!(response.safety.state, PreviewSafetyState::NotApplicable);
        assert!(response.safety.warnings.is_empty());
        assert!(!response.truncated);
        assert_eq!(response.execution_spec_hash.len(), 64);
        assert_eq!(
            response.full_size_bytes,
            response.preview_text.len() as u64 + 3
        );
        assert!(response
            .preview_text
            .contains("Write-Output '中文 空格 user''s value'"));
        assert!(response.preview_text.contains("Write-Output 'enabled'"));
        assert_eq!(response.parameter_summaries.len(), 6);
        let folders = response
            .parameter_summaries
            .iter()
            .find(|summary| summary.parameter_key == "folders")
            .expect("应返回 Folders 摘要");
        assert_eq!(folders.total_count, 2);
        assert_eq!(folders.display_values.len(), 2);
    }

    /// 验证 Boolean false 会稳定移除 if 固定块，而不会跳过其他参数或 each。
    #[test]
    fn false_condition_changes_full_preview_deterministically() {
        let planner = ExecutionPlanner::new();
        let enabled = planner
            .preview(&preview_request(valid_values(true)))
            .expect("true Preview 应成功");
        let disabled = planner
            .preview(&preview_request(valid_values(false)))
            .expect("false Preview 应成功");

        assert!(enabled.preview_text.contains("Write-Output 'enabled'"));
        assert!(!disabled.preview_text.contains("Write-Output 'enabled'"));
        assert_ne!(enabled.execution_spec_hash, disabled.execution_spec_hash);
    }

    /// 验证请求 Map 的构造顺序不影响 Definition 顺序、脚本或完整 Hash。
    #[test]
    fn parameter_map_insertion_order_does_not_change_execution_spec_hash() {
        let planner = ExecutionPlanner::new();
        let first_values = valid_values(true);
        let mut second_values = BTreeMap::new();
        for (key, value) in first_values.iter().rev() {
            second_values.insert(key.clone(), value.clone());
        }

        let first = planner
            .preview(&preview_request(first_values))
            .expect("第一种顺序应成功");
        let second = planner
            .preview(&preview_request(second_values))
            .expect("第二种顺序应成功");

        assert_eq!(first.parameter_summaries, second.parameter_summaries);
        assert_eq!(first.preview_text, second.preview_text);
        assert_eq!(first.execution_spec_hash, second.execution_spec_hash);
    }

    /// 验证 Run 先区分 revision conflict，再区分 stale preview，匹配时只能获得私有授权值。
    #[test]
    fn verify_run_distinguishes_revision_conflict_and_stale_preview() {
        let planner = ExecutionPlanner::new();
        let values = valid_values(true);
        let preview = planner
            .preview(&preview_request(values.clone()))
            .expect("Preview 应成功");
        let revision_error = planner
            .verify_run(&VerifyRunRequest {
                command_block_id: POWERSHELL_PARAMETER_ECHO_ID.to_owned(),
                expected_revision: 2,
                parameter_values: BTreeMap::new(),
                execution_spec_hash: preview.execution_spec_hash.clone(),
                safety_confirmation: None,
                target_identity_hash: None,
            })
            .expect_err("旧 revision 应优先拒绝");
        assert_eq!(revision_error.code, PlannerErrorCode::RevisionConflict);

        let mut changed_values = values.clone();
        changed_values.insert(
            "text".to_owned(),
            ParameterValue::Text("Preview 后变化".to_owned()),
        );
        let stale_error = planner
            .verify_run(&VerifyRunRequest {
                command_block_id: POWERSHELL_PARAMETER_ECHO_ID.to_owned(),
                expected_revision: 1,
                parameter_values: changed_values,
                execution_spec_hash: preview.execution_spec_hash.clone(),
                safety_confirmation: None,
                target_identity_hash: None,
            })
            .expect_err("参数变化后的旧 Hash 应拒绝");
        assert_eq!(stale_error.code, PlannerErrorCode::StalePreview);

        let verified = planner
            .verify_run(&VerifyRunRequest {
                command_block_id: POWERSHELL_PARAMETER_ECHO_ID.to_owned(),
                expected_revision: 1,
                parameter_values: values,
                execution_spec_hash: preview.execution_spec_hash.clone(),
                safety_confirmation: None,
                target_identity_hash: None,
            })
            .expect("当前 revision 与 Hash 应产出 VerifiedExecution");
        let debug = format!("{verified:?}");
        assert!(debug.contains("VerifiedExecution"));
        assert!(!debug.contains("user's value"));
    }

    /// 验证参数错误只返回 key 与稳定原因码，不回显用户路径或文本。
    #[test]
    fn validation_errors_do_not_echo_user_values() {
        let planner = ExecutionPlanner::new();
        let mut values = valid_values(true);
        values.insert(
            "folder".to_owned(),
            ParameterValue::Text(r"Z:\private\missing-user-folder".to_owned()),
        );

        let error = planner
            .preview(&preview_request(values))
            .expect_err("不存在目录应拒绝");
        assert_eq!(error.code, PlannerErrorCode::ValidationFailed);
        assert_eq!(error.parameter_key.as_deref(), Some("folder"));
        assert_eq!(error.detail_code.as_deref(), Some("folderNotFound"));
        assert!(!error.to_string().contains("private"));
    }

    /// 验证 CMD Built-in 通过同一 Planner 接口生成带固定 UTF-8 前导的可信 Preview。
    #[test]
    fn previews_cmd_through_the_same_planner_interface() {
        let planner = ExecutionPlanner::new();
        let values = valid_values(true);
        let preview = planner
            .preview(&PreviewCommandRequest {
                command_block_id: CMD_PARAMETER_ECHO_ID.to_owned(),
                expected_revision: 1,
                parameter_values: values.clone(),
            })
            .expect("CMD Built-in 应生成 Preview");
        let repeated = planner
            .preview(&PreviewCommandRequest {
                command_block_id: CMD_PARAMETER_ECHO_ID.to_owned(),
                expected_revision: 1,
                parameter_values: values.clone(),
            })
            .expect("相同 CMD 输入应再次生成 Preview");

        assert_eq!(preview.runner, RunnerType::Cmd);
        assert!(preview
            .preview_text
            .starts_with("@\"!CMDBOX_INTERNAL_CHCP!\" 65001 >nul\r\n@setlocal EnableExtensions EnableDelayedExpansion\r\n@echo off\r\n"));
        assert_eq!(preview.full_size_bytes, preview.preview_text.len() as u64);
        assert_eq!(preview.execution_spec_hash, repeated.execution_spec_hash);
        planner
            .verify_run(&VerifyRunRequest {
                command_block_id: CMD_PARAMETER_ECHO_ID.to_owned(),
                expected_revision: 1,
                parameter_values: values,
                execution_spec_hash: preview.execution_spec_hash,
                safety_confirmation: None,
                target_identity_hash: None,
            })
            .expect("CMD Run 应复用同一渲染、绑定与 Hash 路径");
    }

    /// 验证 Definition 环境不能以大小写变体覆盖 CMD 私有名称或 SystemRoot。
    #[test]
    fn rejects_cmd_definition_environment_collisions() {
        for reserved_name in ["cmdbox_internal_chcp", "sYsTeMrOoT"] {
            let mut definition = builtin_command_definitions()[1].clone();
            definition
                .environment
                .insert(reserved_name.to_owned(), "attacker".to_owned());
            let error = prepare_execution(definition, &valid_values(true))
                .err()
                .expect("CMD 固定环境不得被 Definition 覆盖");
            assert_eq!(error.code, PlannerErrorCode::InternalContract);
            assert_eq!(
                error.detail_code.as_deref(),
                Some("reservedCmdEnvironmentName")
            );
        }

        let mut definition = builtin_command_definitions()[1].clone();
        definition
            .environment
            .insert("Path".to_owned(), "one".to_owned());
        definition
            .environment
            .insert("PATH".to_owned(), "two".to_owned());
        let error = prepare_execution(definition, &valid_values(true))
            .err()
            .expect("Windows 大小写不敏感重复环境名应拒绝");
        assert_eq!(
            error.detail_code.as_deref(),
            Some("duplicateCmdEnvironmentName")
        );
    }

    /// 验证永久删除 Preview 折叠重复/子目录、绑定身份，只能转换为 Delete Executor 计划。
    #[cfg(feature = "delete-validation")]
    #[test]
    fn delete_preview_binds_effective_targets_and_is_not_launch_ready() {
        let root = IsolatedDeleteRoot::create();
        let parent = root.path().join("parent");
        let child = parent.join("child");
        fs::create_dir_all(&child).expect("应能创建隔离目标层级");
        let parent_text = parent.to_str().expect("测试路径应为 Unicode").to_owned();
        let values = BTreeMap::from([(
            "folders".to_owned(),
            ParameterValue::Array(vec![
                ParameterValue::Text(child.to_str().expect("测试路径应为 Unicode").to_owned()),
                ParameterValue::Text(parent_text.clone()),
                ParameterValue::Text(parent_text),
            ]),
        )]);
        let planner = ExecutionPlanner::new();
        let preview = planner
            .preview(&PreviewCommandRequest {
                command_block_id: DELETE_FOLDERS_ID.to_owned(),
                expected_revision: 1,
                parameter_values: values.clone(),
            })
            .expect("隔离目录应生成永久删除 Preview");

        assert_eq!(preview.risk_level, RiskLevel::Destructive);
        assert_eq!(preview.action_label, "永久删除");
        assert_eq!(preview.safety.state, PreviewSafetyState::Passed);
        assert_eq!(preview.parameter_summaries[0].total_count, 1);
        assert!(preview
            .safety
            .warnings
            .iter()
            .any(|warning| warning.code == "ANCESTOR_COLLAPSED"));
        assert!(preview.confirmation_requirement.is_none());
        let identity_hash = preview
            .target_identity_hash
            .clone()
            .expect("destructive Preview 必须返回目标身份凭据");

        let missing_identity = planner
            .verify_run(&VerifyRunRequest {
                command_block_id: DELETE_FOLDERS_ID.to_owned(),
                expected_revision: 1,
                parameter_values: values.clone(),
                execution_spec_hash: preview.execution_spec_hash.clone(),
                safety_confirmation: None,
                target_identity_hash: None,
            })
            .expect_err("destructive Run 不得省略目标身份凭据");
        assert_eq!(missing_identity.code, PlannerErrorCode::TargetChanged);

        let verified = planner
            .verify_run(&VerifyRunRequest {
                command_block_id: DELETE_FOLDERS_ID.to_owned(),
                expected_revision: 1,
                parameter_values: values.clone(),
                execution_spec_hash: preview.execution_spec_hash.clone(),
                safety_confirmation: None,
                target_identity_hash: Some(identity_hash.clone()),
            })
            .expect("未变化的隔离目标应通过完整 Run 复验");
        assert!(!verified.launch_ready());
        assert!(format!("{verified:?}").contains("collector_protocol_version: Some(1)"));
        assert!(matches!(
            ExecutionManager::new().start(verified),
            Err(ExecutionStartError::ExecutorUnavailable)
        ));
        let delete_verified = planner
            .verify_run(&VerifyRunRequest {
                command_block_id: DELETE_FOLDERS_ID.to_owned(),
                expected_revision: 1,
                parameter_values: values,
                execution_spec_hash: preview.execution_spec_hash,
                safety_confirmation: None,
                target_identity_hash: Some(identity_hash),
            })
            .expect("相同授权应可再次由 Planner 全量复验");
        assert!(
            delete_verified.into_delete_execution_plan().is_some(),
            "删除授权只能整体转换为 Delete Executor 计划"
        );
        assert!(parent.exists(), "仅转换执行计划不得产生删除副作用");
    }

    /// 验证 Preview 后即使同名目录被重建，Run 也会按 File ID 拒绝目标替换。
    #[cfg(feature = "delete-validation")]
    #[test]
    fn delete_run_rejects_recreated_target_identity() {
        let root = IsolatedDeleteRoot::create();
        let target = root.path().join("replace-me");
        fs::create_dir(&target).expect("应能创建隔离目标");
        let values = BTreeMap::from([(
            "folders".to_owned(),
            ParameterValue::Array(vec![ParameterValue::Text(
                target.to_str().expect("测试路径应为 Unicode").to_owned(),
            )]),
        )]);
        let planner = ExecutionPlanner::new();
        let preview = planner
            .preview(&PreviewCommandRequest {
                command_block_id: DELETE_FOLDERS_ID.to_owned(),
                expected_revision: 1,
                parameter_values: values.clone(),
            })
            .expect("原始隔离目标应生成 Preview");
        fs::remove_dir(&target).expect("只移除当前测试拥有的空目标");
        fs::create_dir(&target).expect("应能在同一路径重建不同对象");

        let error = planner
            .verify_run(&VerifyRunRequest {
                command_block_id: DELETE_FOLDERS_ID.to_owned(),
                expected_revision: 1,
                parameter_values: values,
                execution_spec_hash: preview.execution_spec_hash,
                safety_confirmation: None,
                target_identity_hash: preview.target_identity_hash,
            })
            .expect_err("同名重建对象必须被身份复验拒绝");
        assert_eq!(error.code, PlannerErrorCode::TargetChanged);
        assert!(target.exists(), "身份拒绝不得删除重建目标");
    }

    /// 验证卷根在 Preview 阶段即被 Safety Guard 阻断，且错误不回显路径。
    #[cfg(feature = "delete-validation")]
    #[test]
    fn delete_preview_blocks_volume_root() {
        let planner = ExecutionPlanner::new();
        let error = planner
            .preview(&PreviewCommandRequest {
                command_block_id: DELETE_FOLDERS_ID.to_owned(),
                expected_revision: 1,
                parameter_values: BTreeMap::from([(
                    "folders".to_owned(),
                    ParameterValue::Array(vec![ParameterValue::Text(r"C:\".to_owned())]),
                )]),
            })
            .expect_err("卷根必须被阻断");

        assert_eq!(error.code, PlannerErrorCode::SafetyBlocked);
        assert_eq!(error.detail_code.as_deref(), Some("criticalPath"));
        assert!(!error.to_string().contains(r"C:\"));
    }

    /// 验证 high-risk 精确根产生版本化确认要求，且确认短语必须精确匹配。
    #[cfg(feature = "delete-validation")]
    #[test]
    fn high_risk_policy_requires_exact_confirmation_phrase() {
        let root = IsolatedDeleteRoot::create();
        let target = root.path().join("high-risk-root");
        fs::create_dir(&target).expect("应能创建隔离 high-risk 目标");
        let definition = builtin_command_definitions()
            .into_iter()
            .find(|definition| definition.id == DELETE_FOLDERS_ID)
            .expect("应存在永久删除 Definition");
        let values = BTreeMap::from([(
            "folders".to_owned(),
            ParameterValue::Array(vec![ParameterValue::Text(
                target.to_str().expect("测试路径应为 Unicode").to_owned(),
            )]),
        )]);
        let mut normalized = validate_parameter_values(&definition.parameters, &values)
            .expect("隔离目标应通过类型化校验");
        let protected = ProtectedPathSet::explicit(Vec::new(), Vec::new(), vec![target]);
        let (report, confirmation_version, collector_version) =
            apply_safety_policy_with_protected(&definition, &mut normalized, &protected)
                .expect("显式 high-risk 根应允许带强化确认的 Preview");

        assert_eq!(
            report.expect("应返回安全报告").risk,
            DeleteRiskDecision::HighRisk
        );
        assert_eq!(confirmation_version, Some(1));
        assert_eq!(collector_version, Some(1));
        validate_safety_confirmation(None, None).expect("normal 执行不要求确认");
        for response in [
            None,
            Some(super::SafetyConfirmationResponse {
                phrase: "delete".to_owned(),
            }),
        ] {
            let error = validate_safety_confirmation(Some(1), response.as_ref())
                .expect_err("high-risk 必须提交精确 DELETE");
            assert_eq!(error.code, PlannerErrorCode::ConfirmationRequired);
        }
        validate_safety_confirmation(
            Some(1),
            Some(&super::SafetyConfirmationResponse {
                phrase: "DELETE".to_owned(),
            }),
        )
        .expect("精确 DELETE 应满足当前确认契约");
        let unknown_version = validate_safety_confirmation(
            Some(2),
            Some(&super::SafetyConfirmationResponse {
                phrase: "DELETE".to_owned(),
            }),
        )
        .expect_err("未知确认版本必须 fail-closed");
        assert_eq!(unknown_version.code, PlannerErrorCode::InternalContract);
        assert_eq!(
            unknown_version.detail_code.as_deref(),
            Some("unsupportedConfirmationVersion")
        );

        let mut unsupported_definition = definition;
        let SafetyPolicy::DeletePaths {
            confirmation_version,
            collector_protocol_version,
            ..
        } = &mut unsupported_definition.safety_policy
        else {
            panic!("永久删除 Definition 必须声明 DeletePaths");
        };
        *confirmation_version = 2;
        *collector_protocol_version = 2;
        let error = prepare_execution(unsupported_definition, &values)
            .err()
            .expect("未实现的确认与 collector 版本不得进入 Spec");
        assert_eq!(error.code, PlannerErrorCode::InternalContract);
        assert_eq!(error.detail_code.as_deref(), Some("invalidSafetyPolicy"));
    }

    /// 创建只含一个长 Text 的私有 Definition，用于证明展示截断与完整 Artifact Hash 分离。
    fn long_text_definition() -> CommandBlockDefinition {
        CommandBlockDefinition {
            id: "builtin.test.long-preview".to_owned(),
            name: "长 Preview 测试".to_owned(),
            description: "仅用于纯单元测试".to_owned(),
            origin: CommandOrigin::Builtin,
            runner: RunnerType::WindowsPowerShell,
            risk_level: RiskLevel::Normal,
            revision: 1,
            template: "Write-Output {{text}}".to_owned(),
            parameters: vec![ParameterDefinition::Text(TextParameterDefinition {
                base: ParameterBase {
                    key: "text".to_owned(),
                    label: "文本".to_owned(),
                    description: None,
                    required: true,
                    remember: false,
                },
                default_value: None,
                min_length: None,
                max_length: Some(1024),
                placeholder: None,
            })],
            environment: BTreeMap::new(),
            outcome_policy: OutcomePolicy::standard(),
            safety_policy: SafetyPolicy::Generic { version: 1 },
        }
    }

    /// 验证相同截断展示可以对应不同完整 Hash，且 fullSizeBytes 始终统计完整 BOM Artifact。
    #[test]
    fn truncated_preview_still_hashes_complete_artifact() {
        let first_values = BTreeMap::from([(
            "text".to_owned(),
            ParameterValue::Text(format!("{}A", "中文".repeat(200))),
        )]);
        let second_values = BTreeMap::from([(
            "text".to_owned(),
            ParameterValue::Text(format!("{}B", "中文".repeat(200))),
        )]);
        let first = build_preview_response(
            prepare_execution(long_text_definition(), &first_values)
                .expect("第一份长 Artifact 应生成"),
            32,
        )
        .expect("第一份长 Preview 应生成");
        let second = build_preview_response(
            prepare_execution(long_text_definition(), &second_values)
                .expect("第二份长 Artifact 应生成"),
            32,
        )
        .expect("第二份长 Preview 应生成");

        assert!(first.truncated);
        assert!(second.truncated);
        assert_eq!(first.preview_text, second.preview_text);
        assert_ne!(first.execution_spec_hash, second.execution_spec_hash);
        assert!(first.full_size_bytes > first.preview_text.len() as u64 + 3);
        assert!(second.full_size_bytes > second.preview_text.len() as u64 + 3);
    }
}
