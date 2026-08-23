//! CMD-02 使用的固定无破坏 Command Block Definition。
//!
//! 本模块声明 PowerShell 与 CMD 参数回显 Built-in 的稳定身份、Runner、正常风险级别、
//! 类型化参数和受限模板。只有显式 `ui-validation` 构建会额外编译一个无参数短等待
//! Definition；默认应用仍严格只有两个正式 Built-in。Definition 不包含可执行文件、Runner
//! options 或已经渲染的脚本，也不会在构造时创建文件或进程。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::parameter::{
    BooleanParameterDefinition, FolderParameterDefinition, FoldersParameterDefinition,
    NumberParameterDefinition, ParameterBase, ParameterDefinition, SelectParameterDefinition,
    TextParameterDefinition,
};

/// Windows PowerShell 参数回显 Built-in 的稳定 ID。
pub const POWERSHELL_PARAMETER_ECHO_ID: &str = "builtin.parameter-echo.windows-powershell";

/// CMD 参数回显 Built-in 的稳定 ID。
pub const CMD_PARAMETER_ECHO_ID: &str = "builtin.parameter-echo.cmd";

/// 显式真实宿主验收构建使用的固定短等待 Definition ID。
#[cfg(feature = "ui-validation")]
pub const UI_VALIDATION_SHORT_WAIT_ID: &str = "builtin.ui-validation.short-wait";

/// PowerShell 参数回显使用的受限静态模板。
const POWERSHELL_PARAMETER_ECHO_TEMPLATE: &str = "$ErrorActionPreference = 'Stop'\nWrite-Output {{text}}\nWrite-Output {{count}}\n{{#if enabled}}Write-Output 'enabled'\n{{/if}}Write-Output {{mode}}\nWrite-Output {{folder}}\n{{#each folders}}Write-Output {{this}}\n{{/each}}";

/// CMD 参数回显使用的受限静态模板。
const CMD_PARAMETER_ECHO_TEMPLATE: &str = "echo({{text}}\r\necho({{count}}\r\n{{#if enabled}}echo(enabled\r\n{{/if}}echo({{mode}}\r\necho({{folder}}\r\n{{#each folders}}echo({{this}}\r\n{{/each}}";

/// 显式真实宿主验收使用的固定五秒 PowerShell 短等待模板。
#[cfg(feature = "ui-validation")]
const UI_VALIDATION_SHORT_WAIT_TEMPLATE: &str = "Start-Sleep -Seconds 5";

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
        },
    ];
    #[cfg(feature = "ui-validation")]
    let definitions = {
        let mut definitions = definitions;
        definitions.push(ui_validation_short_wait_definition());
        definitions
    };
    definitions
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

    /// 验证默认构建严格只暴露两个正式 Built-in，验证 ID 不能经 Planner 读取。
    #[cfg(not(feature = "ui-validation"))]
    #[test]
    fn excludes_ui_validation_definition_from_default_registry() {
        let planner = ExecutionPlanner::new();

        assert_eq!(planner.list_command_blocks().len(), 2);
        let error = planner
            .get_command_block(UI_VALIDATION_SHORT_WAIT_ID)
            .expect_err("默认 Registry 不得读取验证 Definition");
        assert_eq!(error.code, PlannerErrorCode::CommandBlockNotFound);
    }

    /// 验证显式 feature 只追加一个无参数 PowerShell 短等待，并复用同一 Planner 闭环。
    #[cfg(feature = "ui-validation")]
    #[test]
    fn exposes_ui_validation_definition_through_the_same_planner_flow() {
        let definitions = builtin_command_definitions();
        let validation_definition = &definitions[2];
        assert_eq!(definitions.len(), 3);
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

        assert_eq!(summaries.len(), 3);
        assert_eq!(summaries[0].id, POWERSHELL_PARAMETER_ECHO_ID);
        assert_eq!(summaries[1].id, CMD_PARAMETER_ECHO_ID);
        assert_eq!(summaries[2].id, UI_VALIDATION_SHORT_WAIT_ID);
        assert_eq!(summaries[2].runner, RunnerType::WindowsPowerShell);
        assert_eq!(summaries[2].risk_level, RiskLevel::Normal);

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
            })
            .expect("验证 Definition 应通过同一 Run 复验入口");
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
