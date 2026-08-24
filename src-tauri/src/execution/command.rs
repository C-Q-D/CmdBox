//! CmdBox 固定 Command Block Definition。
//!
//! 本模块声明 PowerShell 与 CMD 参数回显 Built-in 的稳定身份、Runner、正常风险级别、
//! 类型化参数和受限模板。显式 `ui-validation` 构建会追加三条安全验证 Definition；完整
//! Windows 删除门禁通过前，永久删除 Definition 只在 `delete-validation` 构建中存在。
//! Definition 不包含可执行文件、Runner options 或已经渲染的脚本，也不会在构造时创建
//! 文件或进程。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[cfg(feature = "ui-validation")]
use super::outcome::ExitCodeRange;
use super::outcome::OutcomePolicy;
use super::parameter::{
    BooleanParameterDefinition, FolderParameterDefinition, FoldersParameterDefinition,
    NumberParameterDefinition, ParameterBase, ParameterDefinition, SelectParameterDefinition,
    TextParameterDefinition,
};

/// Windows PowerShell 参数回显 Built-in 的稳定 ID。
pub const POWERSHELL_PARAMETER_ECHO_ID: &str = "builtin.parameter-echo.windows-powershell";

/// CMD 参数回显 Built-in 的稳定 ID。
pub const CMD_PARAMETER_ECHO_ID: &str = "builtin.parameter-echo.cmd";

/// 完整 Windows 门禁通过前只存在于 `delete-validation` 的永久删除 Built-in ID。
#[cfg(feature = "delete-validation")]
pub const DELETE_FOLDERS_ID: &str = "builtin.delete-folders.windows-powershell";

/// 显式真实宿主验收构建使用的固定短等待 Definition ID。
#[cfg(feature = "ui-validation")]
pub const UI_VALIDATION_SHORT_WAIT_ID: &str = "builtin.ui-validation.short-wait";

/// 显式真实宿主验收构建使用的普通非零失败 Definition ID。
#[cfg(feature = "ui-validation")]
pub const UI_VALIDATION_ORDINARY_FAILURE_ID: &str = "builtin.ui-validation.ordinary-failure";

/// 显式真实宿主验收构建使用的特殊 Exit Code Definition ID。
#[cfg(feature = "ui-validation")]
pub const UI_VALIDATION_SPECIAL_EXIT_ID: &str = "builtin.ui-validation.special-exit";

/// PowerShell 参数回显使用的受限静态模板。
const POWERSHELL_PARAMETER_ECHO_TEMPLATE: &str = "$ErrorActionPreference = 'Stop'\nWrite-Output {{text}}\nWrite-Output {{count}}\n{{#if enabled}}Write-Output 'enabled'\n{{/if}}Write-Output {{mode}}\nWrite-Output {{folder}}\n{{#each folders}}Write-Output {{this}}\n{{/each}}";

/// CMD 参数回显使用的受限静态模板。
const CMD_PARAMETER_ECHO_TEMPLATE: &str = "echo({{text}}\r\necho({{count}}\r\n{{#if enabled}}echo(enabled\r\n{{/if}}echo({{mode}}\r\necho({{folder}}\r\n{{#each folders}}echo({{this}}\r\n{{/each}}";

/// 永久删除固定模板；Session 在可信 Executor 原子完成前拒绝启动该执行种类。
#[cfg(feature = "delete-validation")]
const DELETE_FOLDERS_TEMPLATE: &str = r#"param(
  [Parameter(Mandatory = $true)][string]$CmdBoxPipe,
  [Parameter(Mandatory = $true)][string]$CmdBoxToken,
  [Parameter(Mandatory = $true)][string]$CmdBoxGeneration
)
$ErrorActionPreference = 'Stop'
$pipe = [System.IO.Pipes.NamedPipeClientStream]::new('.', $CmdBoxPipe, [System.IO.Pipes.PipeDirection]::InOut, [System.IO.Pipes.PipeOptions]::Asynchronous)
try {
  $pipe.Connect(5000)
  $utf8 = [System.Text.UTF8Encoding]::new($false, $true)
  $reader = [System.IO.StreamReader]::new($pipe, $utf8, $false, 1024, $true)
  $writer = [System.IO.StreamWriter]::new($pipe, $utf8, 1024, $true)
  $writer.NewLine = "`n"
  $writer.AutoFlush = $true
  $ioTimeoutMs = 5000
  function Write-CmdBoxLine([string]$line) {
    if ($utf8.GetByteCount($line) -gt 512) { throw 'CmdBox collector request exceeds protocol limit.' }
    $task = $writer.WriteLineAsync($line)
    if (-not $task.Wait($ioTimeoutMs)) { throw 'CmdBox collector write timeout.' }
    $task.GetAwaiter().GetResult()
  }
  function Read-CmdBoxLine {
    $task = $reader.ReadLineAsync()
    if (-not $task.Wait($ioTimeoutMs)) { throw 'CmdBox collector read timeout.' }
    $line = $task.GetAwaiter().GetResult()
    if ($null -eq $line -or $utf8.GetByteCount($line) -gt 512) { throw 'CmdBox collector response is invalid.' }
    return $line
  }
  $index = 0
  $failed = 0
{{#each folders}}  Write-CmdBoxLine (('BEGIN|{0}|{1}|{2}' -f $CmdBoxToken, $CmdBoxGeneration, $index))
  $approval = Read-CmdBoxLine
  if ($approval -ne ('APPROVE|{0}|{1}|{2}' -f $CmdBoxToken, $CmdBoxGeneration, $index)) { exit 70 }
  try {
    Remove-Item -LiteralPath {{this}} -Recurse -Force -ErrorAction Stop
    if (Test-Path -LiteralPath {{this}}) {
      Write-CmdBoxLine (('FAILURE|{0}|{1}|{2}|stillExists' -f $CmdBoxToken, $CmdBoxGeneration, $index))
      $failed++
    } else {
      Write-CmdBoxLine (('SUCCESS|{0}|{1}|{2}' -f $CmdBoxToken, $CmdBoxGeneration, $index))
    }
  } catch {
    Write-CmdBoxLine (('FAILURE|{0}|{1}|{2}|removeFailed' -f $CmdBoxToken, $CmdBoxGeneration, $index))
    $failed++
  }
  $ack = Read-CmdBoxLine
  if ($ack -ne ('ACK|{0}|{1}|{2}' -f $CmdBoxToken, $CmdBoxGeneration, $index)) { exit 71 }
  $index++
{{/each}}  if ($failed -gt 0) { exit 1 }
  exit 0
} finally {
  if ($null -ne $writer) { $writer.Dispose() }
  if ($null -ne $reader) { $reader.Dispose() }
  $pipe.Dispose()
}"#;

/// Command Definition 声明的结构化 Safety Policy。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub(crate) enum SafetyPolicy {
    /// normal Command 不操作破坏性路径。
    Generic { version: u32 },
    /// 永久删除只接受指定 Folders 参数，并绑定强化确认与 collector 协议。
    DeletePaths {
        version: u32,
        parameter_key: String,
        confirmation_version: u32,
        collector_protocol_version: u32,
    },
}

impl SafetyPolicy {
    /// 返回进入 Canonical Execution Spec 的策略版本。
    pub(crate) const fn version(&self) -> u32 {
        match self {
            Self::Generic { version } | Self::DeletePaths { version, .. } => *version,
        }
    }
}

/// 显式真实宿主验收使用的固定五秒 PowerShell 短等待模板。
#[cfg(feature = "ui-validation")]
const UI_VALIDATION_SHORT_WAIT_TEMPLATE: &str = "Start-Sleep -Seconds 5";

/// 显式真实宿主验收使用的固定普通失败模板。
#[cfg(feature = "ui-validation")]
const UI_VALIDATION_ORDINARY_FAILURE_TEMPLATE: &str = "exit 9";

/// 显式真实宿主验收使用的参数化特殊 Exit Code 模板。
#[cfg(feature = "ui-validation")]
const UI_VALIDATION_SPECIAL_EXIT_TEMPLATE: &str = "exit {{exitCode}}";

/// Command Block 的来源身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase")]
pub enum CommandOrigin {
    /// 由 CmdBox 固定提供并测试的 Built-in Definition。
    Builtin,
    /// 后续持久化单元创建的用户 Definition；当前两个固定命令不会使用该值。
    User,
}

/// 当前 Windows MVP 可由 Command Block 声明的 Runner。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase")]
pub enum RunnerType {
    /// 系统 Windows PowerShell Runner。
    WindowsPowerShell,
    /// 系统 CMD Runner，通过确定 Artifact、私有环境绑定与真实编码门禁执行。
    Cmd,
}

/// Command Block 的稳定风险语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export_to = "contracts.ts"))]
#[serde(rename_all = "camelCase")]
pub enum RiskLevel {
    /// 不产生不可逆文件副作用的普通命令。
    Normal,
    /// 后续安全单元使用的破坏性命令身份。
    Destructive,
}

/// Rust Core 提供给后续 Preview 流程的不可变 Command Block Definition。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommandBlockDefinition {
    /// Command Block 的稳定身份。
    pub id: String,
    /// Command Workspace 展示的名称。
    pub name: String,
    /// Command Workspace 展示的用途说明。
    pub description: String,
    /// Built-in 或 User 来源身份。
    pub origin: CommandOrigin,
    /// 后续 Preview/Run 必须使用的声明 Runner。
    pub runner: RunnerType,
    /// 当前 Command Block 的正常或破坏性风险语义。
    pub risk_level: RiskLevel,
    /// Definition 发生语义变化时递增的 revision。
    pub revision: u64,
    /// 只允许由 Rust 受限 Parser 解释的静态模板。
    pub template: String,
    /// 按 Command Workspace 与规范化输出顺序排列的 Parameter Definition。
    pub parameters: Vec<ParameterDefinition>,
    /// Runner 继承环境之外由 Definition 明确声明的非敏感环境变量。
    pub environment: BTreeMap<String, String>,
    /// Rust Core 用于解释自然终态或类型化目标事实的版本化策略。
    pub(crate) outcome_policy: OutcomePolicy,
    /// Rust Core 在渲染与启动前执行的结构化 Safety Policy。
    pub(crate) safety_policy: SafetyPolicy,
}

/// 按稳定顺序返回正式 Built-in，并仅在显式验证 feature 下追加短等待 Definition。
pub(crate) fn builtin_command_definitions() -> Vec<CommandBlockDefinition> {
    let definitions = vec![
        CommandBlockDefinition {
            id: POWERSHELL_PARAMETER_ECHO_ID.to_owned(),
            name: "PowerShell 参数回显".to_owned(),
            description: "用 Windows PowerShell 回显六类类型化参数。".to_owned(),
            origin: CommandOrigin::Builtin,
            runner: RunnerType::WindowsPowerShell,
            risk_level: RiskLevel::Normal,
            revision: 1,
            template: POWERSHELL_PARAMETER_ECHO_TEMPLATE.to_owned(),
            parameters: echo_parameter_definitions(),
            environment: BTreeMap::new(),
            outcome_policy: OutcomePolicy::standard(),
            safety_policy: SafetyPolicy::Generic { version: 1 },
        },
        CommandBlockDefinition {
            id: CMD_PARAMETER_ECHO_ID.to_owned(),
            name: "CMD 参数回显".to_owned(),
            description: "用 CMD 回显六类类型化参数。".to_owned(),
            origin: CommandOrigin::Builtin,
            runner: RunnerType::Cmd,
            risk_level: RiskLevel::Normal,
            revision: 1,
            template: CMD_PARAMETER_ECHO_TEMPLATE.to_owned(),
            parameters: echo_parameter_definitions(),
            environment: BTreeMap::new(),
            outcome_policy: OutcomePolicy::standard(),
            safety_policy: SafetyPolicy::Generic { version: 1 },
        },
    ];
    #[cfg(feature = "ui-validation")]
    let definitions = {
        let mut definitions = definitions;
        definitions.push(ui_validation_short_wait_definition());
        definitions.push(ui_validation_ordinary_failure_definition());
        definitions.push(ui_validation_special_exit_definition());
        definitions
    };
    #[cfg(feature = "delete-validation")]
    let definitions = {
        let mut definitions = definitions;
        definitions.push(delete_folders_definition());
        definitions
    };
    definitions
}

/// 创建只在安全门禁构建存在的永久删除 Definition。
#[cfg(feature = "delete-validation")]
fn delete_folders_definition() -> CommandBlockDefinition {
    CommandBlockDefinition {
        id: DELETE_FOLDERS_ID.to_owned(),
        name: "快速永久删除多个文件夹".to_owned(),
        description: "永久删除所选文件夹，不经过回收站。".to_owned(),
        origin: CommandOrigin::Builtin,
        runner: RunnerType::WindowsPowerShell,
        risk_level: RiskLevel::Destructive,
        revision: 1,
        template: DELETE_FOLDERS_TEMPLATE.to_owned(),
        parameters: vec![ParameterDefinition::Folders(FoldersParameterDefinition {
            base: parameter_base("folders", "要永久删除的文件夹"),
            must_exist: true,
            min_items: Some(1),
            max_items: Some(32),
            default_value: None,
        })],
        environment: BTreeMap::new(),
        outcome_policy: OutcomePolicy::target_results(1),
        safety_policy: SafetyPolicy::DeletePaths {
            version: super::safety::DELETE_PATH_POLICY_VERSION,
            parameter_key: "folders".to_owned(),
            confirmation_version: 1,
            collector_protocol_version: 1,
        },
    }
}

/// 创建只存在于显式验证构建、复用普通 Planner/Runner 的无参数短等待 Definition。
#[cfg(feature = "ui-validation")]
fn ui_validation_short_wait_definition() -> CommandBlockDefinition {
    CommandBlockDefinition {
        id: UI_VALIDATION_SHORT_WAIT_ID.to_owned(),
        name: "验证：短等待".to_owned(),
        description: "用于真实宿主 Cancel 验收的固定五秒等待。".to_owned(),
        origin: CommandOrigin::Builtin,
        runner: RunnerType::WindowsPowerShell,
        risk_level: RiskLevel::Normal,
        revision: 1,
        template: UI_VALIDATION_SHORT_WAIT_TEMPLATE.to_owned(),
        parameters: Vec::new(),
        environment: BTreeMap::new(),
        outcome_policy: OutcomePolicy::standard(),
        safety_policy: SafetyPolicy::Generic { version: 1 },
    }
}

/// 创建只存在于显式验证构建的普通非零失败 Definition。
#[cfg(feature = "ui-validation")]
fn ui_validation_ordinary_failure_definition() -> CommandBlockDefinition {
    CommandBlockDefinition {
        id: UI_VALIDATION_ORDINARY_FAILURE_ID.to_owned(),
        name: "验证：普通失败".to_owned(),
        description: "用于真实宿主普通非零失败 Outcome 验收。".to_owned(),
        origin: CommandOrigin::Builtin,
        runner: RunnerType::WindowsPowerShell,
        risk_level: RiskLevel::Normal,
        revision: 1,
        template: UI_VALIDATION_ORDINARY_FAILURE_TEMPLATE.to_owned(),
        parameters: Vec::new(),
        environment: BTreeMap::new(),
        outcome_policy: OutcomePolicy::standard(),
        safety_policy: SafetyPolicy::Generic { version: 1 },
    }
}

/// 创建只存在于显式验证构建的特殊非零成功与警告 Definition。
#[cfg(feature = "ui-validation")]
fn ui_validation_special_exit_definition() -> CommandBlockDefinition {
    CommandBlockDefinition {
        id: UI_VALIDATION_SPECIAL_EXIT_ID.to_owned(),
        name: "验证：特殊退出码".to_owned(),
        description: "用于真实宿主非零成功、警告与失败 Outcome 验收。".to_owned(),
        origin: CommandOrigin::Builtin,
        runner: RunnerType::WindowsPowerShell,
        risk_level: RiskLevel::Normal,
        revision: 1,
        template: UI_VALIDATION_SPECIAL_EXIT_TEMPLATE.to_owned(),
        parameters: vec![ParameterDefinition::Select(SelectParameterDefinition {
            base: parameter_base("exitCode", "退出码"),
            options: vec!["1".to_owned(), "3".to_owned(), "8".to_owned()],
            default_value: Some("1".to_owned()),
        })],
        environment: BTreeMap::new(),
        outcome_policy: OutcomePolicy::exit_code(
            1,
            vec![ExitCodeRange { start: 0, end: 1 }],
            vec![ExitCodeRange { start: 2, end: 7 }],
        ),
        safety_policy: SafetyPolicy::Generic { version: 1 },
    }
}

/// 创建两个回显 Built-in 共享但彼此独立拥有的六类 Parameter Definition。
fn echo_parameter_definitions() -> Vec<ParameterDefinition> {
    vec![
        ParameterDefinition::Text(TextParameterDefinition {
            base: parameter_base("text", "文本"),
            default_value: None,
            min_length: Some(1),
            max_length: Some(256),
            placeholder: Some("输入要回显的文本".to_owned()),
        }),
        ParameterDefinition::Number(NumberParameterDefinition {
            base: parameter_base("count", "数字"),
            default_value: Some(2.0),
            min: Some(0.0),
            max: Some(100.0),
            step: Some(1.0),
        }),
        ParameterDefinition::Boolean(BooleanParameterDefinition {
            base: parameter_base("enabled", "启用条件输出"),
            default_value: true,
        }),
        ParameterDefinition::Select(SelectParameterDefinition {
            base: parameter_base("mode", "回显模式"),
            options: vec!["brief".to_owned(), "detailed".to_owned()],
            default_value: Some("brief".to_owned()),
        }),
        ParameterDefinition::Folder(FolderParameterDefinition {
            base: parameter_base("folder", "单个目录"),
            must_exist: true,
            default_value: None,
        }),
        ParameterDefinition::Folders(FoldersParameterDefinition {
            base: parameter_base("folders", "多个目录"),
            must_exist: true,
            min_items: Some(1),
            max_items: Some(8),
            default_value: None,
        }),
    ]
}

/// 创建固定回显 Built-in 的 required、不可记忆参数元数据。
fn parameter_base(key: &str, label: &str) -> ParameterBase {
    ParameterBase {
        key: key.to_owned(),
        label: label.to_owned(),
        description: None,
        required: true,
        remember: false,
    }
}

#[cfg(test)]
mod tests {
    //! 两个固定 Built-in 的身份、风险、参数顺序、值校验与模板 AST 测试。

    use std::collections::BTreeMap;

    use super::{
        builtin_command_definitions, CommandOrigin, RiskLevel, RunnerType, CMD_PARAMETER_ECHO_ID,
        POWERSHELL_PARAMETER_ECHO_ID,
    };
    #[cfg(feature = "delete-validation")]
    use super::{SafetyPolicy, DELETE_FOLDERS_ID};
    #[cfg(feature = "ui-validation")]
    use crate::execution::outcome::{Outcome, OutcomePolicy};
    use crate::execution::parameter::{validate_parameter_values, ParameterKind, ParameterValue};
    use crate::execution::planner::ExecutionPlanner;
    #[cfg(not(feature = "ui-validation"))]
    use crate::execution::planner::PlannerErrorCode;
    #[cfg(feature = "ui-validation")]
    use crate::execution::planner::{PreviewCommandRequest, VerifyRunRequest};
    use crate::execution::template::parse_template;

    /// 显式验证构建使用的固定无参数短等待 Definition ID。
    const UI_VALIDATION_SHORT_WAIT_ID: &str = "builtin.ui-validation.short-wait";

    /// 把当前平台绝对目录转换为结构化 Folder 参数文本。
    fn existing_folder_value() -> String {
        std::env::temp_dir().to_string_lossy().into_owned()
    }

    /// 创建两个 Built-in 都应接受的完整六类结构化值。
    fn valid_echo_values() -> BTreeMap<String, ParameterValue> {
        let folder = existing_folder_value();
        BTreeMap::from([
            (
                "text".to_owned(),
                ParameterValue::Text("中文 空格 ' 单引号".to_owned()),
            ),
            ("count".to_owned(), ParameterValue::Number(4.0)),
            ("enabled".to_owned(), ParameterValue::Boolean(true)),
            (
                "mode".to_owned(),
                ParameterValue::Text("detailed".to_owned()),
            ),
            ("folder".to_owned(), ParameterValue::Text(folder.clone())),
            (
                "folders".to_owned(),
                ParameterValue::Array(vec![
                    ParameterValue::Text(folder.clone()),
                    ParameterValue::Text(folder),
                ]),
            ),
        ])
    }

    /// 验证两个正式 Definition 在所有 feature 组合下保持稳定身份、顺序和参数契约。
    #[test]
    fn keeps_two_stable_normal_risk_echo_definitions() {
        let definitions = builtin_command_definitions();
        let echo_definitions = definitions
            .iter()
            .filter(|definition| {
                matches!(
                    definition.id.as_str(),
                    POWERSHELL_PARAMETER_ECHO_ID | CMD_PARAMETER_ECHO_ID
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(echo_definitions.len(), 2);
        assert_eq!(echo_definitions[0].id, POWERSHELL_PARAMETER_ECHO_ID);
        assert_eq!(echo_definitions[0].runner, RunnerType::WindowsPowerShell);
        assert_eq!(echo_definitions[1].id, CMD_PARAMETER_ECHO_ID);
        assert_eq!(echo_definitions[1].runner, RunnerType::Cmd);
        for definition in echo_definitions {
            assert_eq!(definition.origin, CommandOrigin::Builtin);
            assert_eq!(definition.risk_level, RiskLevel::Normal);
            assert_eq!(definition.revision, 1);
            assert_eq!(definition.outcome_policy.version(), 1);
            assert_eq!(
                definition
                    .parameters
                    .iter()
                    .map(|parameter| parameter.kind())
                    .collect::<Vec<_>>(),
                vec![
                    ParameterKind::Text,
                    ParameterKind::Number,
                    ParameterKind::Boolean,
                    ParameterKind::Select,
                    ParameterKind::Folder,
                    ParameterKind::Folders,
                ]
            );
        }
    }

    /// 验证永久删除 Definition 只在门禁 feature 中出现且保持单一 Folders 输入契约。
    #[cfg(feature = "delete-validation")]
    #[test]
    fn exposes_feature_gated_delete_definition() {
        let definitions = builtin_command_definitions();
        let definition = definitions
            .iter()
            .find(|definition| definition.id == DELETE_FOLDERS_ID)
            .expect("delete-validation 应追加永久删除 Definition");

        assert_eq!(definition.origin, CommandOrigin::Builtin);
        assert_eq!(definition.runner, RunnerType::WindowsPowerShell);
        assert_eq!(definition.risk_level, RiskLevel::Destructive);
        assert_eq!(definition.parameters.len(), 1);
        assert_eq!(definition.parameters[0].kind(), ParameterKind::Folders);
        assert!(matches!(
            &definition.safety_policy,
            SafetyPolicy::DeletePaths {
                parameter_key,
                confirmation_version: 1,
                collector_protocol_version: 1,
                ..
            } if parameter_key == "folders"
        ));
    }

    /// 验证默认构建严格只暴露两个正式 Built-in，验证 ID 不能经 Planner 读取。
    #[cfg(not(feature = "ui-validation"))]
    #[test]
    fn excludes_ui_validation_definition_from_default_registry() {
        let planner = ExecutionPlanner::new();

        assert_eq!(
            planner.list_command_blocks().len(),
            2 + usize::from(cfg!(feature = "delete-validation"))
        );
        let error = planner
            .get_command_block(UI_VALIDATION_SHORT_WAIT_ID)
            .expect_err("默认 Registry 不得读取验证 Definition");
        assert_eq!(error.code, PlannerErrorCode::CommandBlockNotFound);
    }

    /// 验证显式 feature 追加三个安全验证 Definition，并复用同一 Planner 闭环。
    #[cfg(feature = "ui-validation")]
    #[test]
    fn exposes_ui_validation_definition_through_the_same_planner_flow() {
        let definitions = builtin_command_definitions();
        let validation_definition = &definitions[2];
        assert_eq!(
            definitions.len(),
            5 + usize::from(cfg!(feature = "delete-validation"))
        );
        assert_eq!(validation_definition.id, UI_VALIDATION_SHORT_WAIT_ID);
        assert_eq!(validation_definition.origin, CommandOrigin::Builtin);
        assert_eq!(validation_definition.runner, RunnerType::WindowsPowerShell);
        assert_eq!(validation_definition.risk_level, RiskLevel::Normal);
        assert_eq!(validation_definition.revision, 1);
        assert_eq!(validation_definition.template, "Start-Sleep -Seconds 5");
        assert!(validation_definition.parameters.is_empty());
        assert!(validation_definition.environment.is_empty());

        let planner = ExecutionPlanner::new();
        let summaries = planner.list_command_blocks();

        assert_eq!(
            summaries.len(),
            5 + usize::from(cfg!(feature = "delete-validation"))
        );
        assert_eq!(summaries[0].id, POWERSHELL_PARAMETER_ECHO_ID);
        assert_eq!(summaries[1].id, CMD_PARAMETER_ECHO_ID);
        assert_eq!(summaries[2].id, UI_VALIDATION_SHORT_WAIT_ID);
        assert_eq!(summaries[2].runner, RunnerType::WindowsPowerShell);
        assert_eq!(summaries[2].risk_level, RiskLevel::Normal);
        assert_eq!(summaries[3].id, "builtin.ui-validation.ordinary-failure");
        assert_eq!(summaries[4].id, "builtin.ui-validation.special-exit");

        let details = planner
            .get_command_block(UI_VALIDATION_SHORT_WAIT_ID)
            .expect("验证 Definition 应通过同一详情入口读取");
        assert!(details.parameters.is_empty());
        let request = PreviewCommandRequest {
            command_block_id: UI_VALIDATION_SHORT_WAIT_ID.to_owned(),
            expected_revision: details.revision,
            parameter_values: BTreeMap::new(),
        };
        let preview = planner
            .preview(&request)
            .expect("验证 Definition 应通过同一 Preview 入口");
        assert_eq!(preview.preview_text, "Start-Sleep -Seconds 5");
        assert_eq!(preview.risk_level, RiskLevel::Normal);
        planner
            .verify_run(&VerifyRunRequest {
                command_block_id: request.command_block_id,
                expected_revision: request.expected_revision,
                parameter_values: BTreeMap::new(),
                execution_spec_hash: preview.execution_spec_hash,
                safety_confirmation: None,
                target_identity_hash: None,
            })
            .expect("验证 Definition 应通过同一 Run 复验入口");

        let ordinary_failure = &definitions[3];
        assert_eq!(ordinary_failure.template, "exit 9");
        assert_eq!(
            ordinary_failure.outcome_policy.interpret_exit_code(9),
            Outcome::Failure
        );
        let special_exit = &definitions[4];
        assert_eq!(special_exit.parameters.len(), 1);
        assert_eq!(
            special_exit.outcome_policy.interpret_exit_code(1),
            Outcome::Success
        );
        assert_eq!(
            special_exit.outcome_policy.interpret_exit_code(3),
            Outcome::Warning
        );
        assert_eq!(
            special_exit.outcome_policy.interpret_exit_code(8),
            Outcome::Failure
        );
        assert_eq!(
            special_exit.outcome_policy,
            OutcomePolicy::exit_code(
                1,
                vec![crate::execution::outcome::ExitCodeRange { start: 0, end: 1 },],
                vec![crate::execution::outcome::ExitCodeRange { start: 2, end: 7 },]
            )
        );

        let special_details = planner
            .get_command_block(&special_exit.id)
            .expect("特殊 Exit Code Definition 应通过详情入口读取");
        let special_request = PreviewCommandRequest {
            command_block_id: special_details.id,
            expected_revision: special_details.revision,
            parameter_values: BTreeMap::from([(
                "exitCode".to_owned(),
                ParameterValue::Text("3".to_owned()),
            )]),
        };
        let special_preview = planner
            .preview(&special_request)
            .expect("特殊 Exit Code Definition 应通过 Preview");
        assert_eq!(special_preview.preview_text, "exit '3'");
        planner
            .verify_run(&VerifyRunRequest {
                command_block_id: special_request.command_block_id,
                expected_revision: special_request.expected_revision,
                parameter_values: special_request.parameter_values,
                execution_spec_hash: special_preview.execution_spec_hash,
                safety_confirmation: None,
                target_identity_hash: None,
            })
            .expect("特殊 Exit Code Definition 应通过同一 Run 复验入口");
    }

    /// 验证两个 Built-in 的完整六类值与受限模板都能进入确定的后续语义。
    #[test]
    fn builtin_parameters_and_templates_are_self_consistent() {
        let values = valid_echo_values();
        let definitions = builtin_command_definitions();
        let echo_definitions = definitions.iter().filter(|definition| {
            matches!(
                definition.id.as_str(),
                POWERSHELL_PARAMETER_ECHO_ID | CMD_PARAMETER_ECHO_ID
            )
        });
        for definition in echo_definitions {
            let normalized = validate_parameter_values(&definition.parameters, &values)
                .expect("Built-in 六类值应通过");
            assert_eq!(normalized.entries.len(), 6);

            let ast = parse_template(&definition.template, &definition.parameters)
                .expect("Built-in 模板应符合受限语法");
            assert!(!ast.nodes.is_empty());
        }
    }
}
