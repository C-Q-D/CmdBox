//! CMD-02 使用的固定无破坏 Command Block Definition。
//!
//! 本模块只声明 PowerShell 与 CMD 参数回显 Built-in 的稳定身份、Runner、正常风险级别、
//! 类型化参数和受限模板。Definition 不包含可执行文件、Runner options 或已经渲染的脚本，
//! 也不会在构造时创建文件或进程。

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

/// PowerShell 参数回显使用的受限静态模板。
const POWERSHELL_PARAMETER_ECHO_TEMPLATE: &str = "$ErrorActionPreference = 'Stop'\nWrite-Output {{text}}\nWrite-Output {{count}}\n{{#if enabled}}Write-Output 'enabled'\n{{/if}}Write-Output {{mode}}\nWrite-Output {{folder}}\n{{#each folders}}Write-Output {{this}}\n{{/each}}";

/// CMD 参数回显使用的受限静态模板。
const CMD_PARAMETER_ECHO_TEMPLATE: &str = "echo({{text}}\r\necho({{count}}\r\n{{#if enabled}}echo(enabled\r\n{{/if}}echo({{mode}}\r\necho({{folder}}\r\n{{#each folders}}echo({{this}}\r\n{{/each}}";

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

/// 按稳定顺序返回 PowerShell 和 CMD 两个正常风险参数回显 Built-in。
pub(crate) fn builtin_command_definitions() -> [CommandBlockDefinition; 2] {
    [
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
    ]
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
    use crate::execution::template::parse_template;

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

    /// 验证固定 Definition 的稳定顺序、Built-in 身份、normal 风险和声明 Runner。
    #[test]
    fn exposes_two_stable_normal_risk_builtin_definitions() {
        let definitions = builtin_command_definitions();

        assert_eq!(definitions[0].id, POWERSHELL_PARAMETER_ECHO_ID);
        assert_eq!(definitions[0].runner, RunnerType::WindowsPowerShell);
        assert_eq!(definitions[1].id, CMD_PARAMETER_ECHO_ID);
        assert_eq!(definitions[1].runner, RunnerType::Cmd);
        for definition in &definitions {
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

    /// 验证两个 Built-in 的完整六类值与受限模板都能进入确定的后续语义。
    #[test]
    fn builtin_parameters_and_templates_are_self_consistent() {
        let values = valid_echo_values();
        for definition in builtin_command_definitions() {
            let normalized = validate_parameter_values(&definition.parameters, &values)
                .expect("Built-in 六类值应通过");
            assert_eq!(normalized.entries.len(), 6);

            let ast = parse_template(&definition.template, &definition.parameters)
                .expect("Built-in 模板应符合受限语法");
            assert!(!ast.nodes.is_empty());
        }
    }
}
