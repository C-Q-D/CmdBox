//! Windows PowerShell 与 CMD 受限模板的确定性序列化与渲染。
//!
//! 本模块只消费已经由 Parameter Validator 规范化的值和已通过引用校验的 Template AST，
//! PowerShell 把业务值编码为单引号字面量；CMD 只接受固定 `echo(` 行级语法，并把非空值
//! 绑定为私有环境变量引用。它不读取文件、不解析任意表达式，也不启动进程；最终产物统一
//! 交给 `RenderedScript` 冻结为 Runner 所需的完整编码字节。

use std::collections::BTreeMap;
use std::error::Error;
use std::ffi::OsString;
use std::fmt::{Display, Formatter};

use super::artifact::RenderedScript;
use super::parameter::{NormalizedParameterValue, NormalizedParameters};
use super::template::{TemplateAst, TemplateNode, TemplateValueSource};
use crate::process::windows::runner::CMD_CHCP_ENVIRONMENT_NAME;

/// CMD 参数字面绑定使用的内部保留前缀；配置环境不得占用该命名空间。
pub(crate) const CMD_VALUE_ENVIRONMENT_PREFIX: &str = "CMDBOX_INTERNAL_VALUE_";

/// CMD 单变量值和展开后物理命令行的排他 UTF-16 上限。
const CMD_MAX_UTF16_UNITS: usize = 8191;

/// 已通过 AST 校验但无法从规范化参数环境取到所需类型时的内部渲染错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PowerShellRenderErrorCode {
    /// AST 引用的参数 key 不存在于规范化结果。
    MissingParameter,
    /// `if` 节点没有读取到 Boolean 值。
    InvalidIfValue,
    /// `each` 节点没有读取到 Folders 值。
    InvalidEachValue,
    /// `this` 节点脱离了当前 `each` 迭代项。
    MissingEachItem,
}

impl PowerShellRenderErrorCode {
    /// 返回不包含参数值的稳定开发者错误标识。
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::MissingParameter => "missingParameter",
            Self::InvalidIfValue => "invalidIfValue",
            Self::InvalidEachValue => "invalidEachValue",
            Self::MissingEachItem => "missingEachItem",
        }
    }
}

/// PowerShell 渲染阶段的窄内部错误，不携带任何用户参数值或本机路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PowerShellRenderError {
    /// 可稳定记录的失败原因。
    pub(crate) code: PowerShellRenderErrorCode,
    /// 与失败直接相关的参数 key；`this` 失败时为空。
    pub(crate) parameter_key: Option<String>,
}

impl PowerShellRenderError {
    /// 创建一个不回显参数内容的渲染错误。
    fn new(code: PowerShellRenderErrorCode, parameter_key: Option<&str>) -> Self {
        Self {
            code,
            parameter_key: parameter_key.map(ToOwned::to_owned),
        }
    }
}

/// 输出不包含参数值的稳定渲染错误说明。
impl Display for PowerShellRenderError {
    /// 格式化不回显参数内容的 PowerShell 渲染错误。
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.parameter_key {
            Some(key) => write!(
                formatter,
                "PowerShell 参数 {key} 渲染失败：{}",
                self.code.as_str()
            ),
            None => write!(formatter, "PowerShell 模板渲染失败：{}", self.code.as_str()),
        }
    }
}

/// PowerShell 渲染错误没有需要向外暴露的底层错误来源。
impl Error for PowerShellRenderError {}

/// 已完成字面量序列化的可读脚本文本与最终 BOM Artifact。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderedPowerShell {
    /// 不含 BOM、供 Preview 文本展示的完整 PowerShell 源码。
    pub(crate) script_text: String,
    /// 含 UTF-8 BOM、供完整 Hash 和后续 Execution 使用的最终脚本。
    pub(crate) artifact: RenderedScript,
}

/// 通过唯一 PowerShell Serializer 路径渲染 AST，并冻结最终 UTF-8 BOM 字节。
pub(crate) fn render_windows_powershell(
    ast: &TemplateAst,
    parameters: &NormalizedParameters,
) -> Result<RenderedPowerShell, PowerShellRenderError> {
    let mut script_text = String::new();
    render_nodes(&ast.nodes, parameters, None, &mut script_text)?;
    let artifact = RenderedScript::windows_powershell(&script_text);
    Ok(RenderedPowerShell {
        script_text,
        artifact,
    })
}

/// 把任意 Unicode 文本编码为 PowerShell 单引号字面量，内部单引号按语言规则成对转义。
fn serialize_single_quoted_literal(value: &str) -> String {
    let mut literal = String::with_capacity(value.len().saturating_add(2));
    literal.push('\'');
    for character in value.chars() {
        if character == '\'' {
            literal.push('\'');
        }
        literal.push(character);
    }
    literal.push('\'');
    literal
}

/// 把一个已经规范化的值编码为不会引入模板结构的 PowerShell 字面量。
fn serialize_parameter_value(value: &NormalizedParameterValue) -> String {
    match value {
        NormalizedParameterValue::Text(value)
        | NormalizedParameterValue::Select(value)
        | NormalizedParameterValue::Folder(value) => serialize_single_quoted_literal(value),
        NormalizedParameterValue::Number(value) => serialize_number(*value),
        NormalizedParameterValue::Boolean(value) => {
            if *value {
                "$true".to_owned()
            } else {
                "$false".to_owned()
            }
        }
        NormalizedParameterValue::Folders(values) => {
            let literals = values
                .iter()
                .map(|value| serialize_single_quoted_literal(value))
                .collect::<Vec<_>>()
                .join(", ");
            format!("@({literals})")
        }
    }
}

/// 把有限 Number 转为 PowerShell 可解析的稳定十进制文本，并统一正负零。
fn serialize_number(value: f64) -> String {
    if value == 0.0 {
        "0".to_owned()
    } else {
        value.to_string()
    }
}

/// 按 AST 顺序递归渲染节点；当前 `each` 项只在对应循环体内可见。
fn render_nodes(
    nodes: &[TemplateNode],
    parameters: &NormalizedParameters,
    each_item: Option<&str>,
    output: &mut String,
) -> Result<(), PowerShellRenderError> {
    for node in nodes {
        match node {
            TemplateNode::Text { value } => output.push_str(value),
            TemplateNode::Value { source } => {
                render_value(source, parameters, each_item, output)?;
            }
            TemplateNode::If { key, body } => match parameters.get(key) {
                Some(NormalizedParameterValue::Boolean(true)) => {
                    render_nodes(body, parameters, each_item, output)?;
                }
                Some(NormalizedParameterValue::Boolean(false)) | None => {}
                Some(_) => {
                    return Err(PowerShellRenderError::new(
                        PowerShellRenderErrorCode::InvalidIfValue,
                        Some(key),
                    ));
                }
            },
            TemplateNode::Each { key, body } => match parameters.get(key) {
                Some(NormalizedParameterValue::Folders(values)) => {
                    for value in values {
                        render_nodes(body, parameters, Some(value), output)?;
                    }
                }
                None => {}
                Some(_) => {
                    return Err(PowerShellRenderError::new(
                        PowerShellRenderErrorCode::InvalidEachValue,
                        Some(key),
                    ));
                }
            },
        }
    }
    Ok(())
}

/// 渲染一个 Parameter 或 `this` 值节点，optional 缺省值使用 PowerShell `$null`。
fn render_value(
    source: &TemplateValueSource,
    parameters: &NormalizedParameters,
    each_item: Option<&str>,
    output: &mut String,
) -> Result<(), PowerShellRenderError> {
    match source {
        TemplateValueSource::Parameter { key } => {
            let Some(entry) = parameters.entries.iter().find(|entry| entry.key == *key) else {
                return Err(PowerShellRenderError::new(
                    PowerShellRenderErrorCode::MissingParameter,
                    Some(key),
                ));
            };
            match &entry.value {
                Some(value) => output.push_str(&serialize_parameter_value(value)),
                None => output.push_str("$null"),
            }
        }
        TemplateValueSource::EachItem => {
            let Some(value) = each_item else {
                return Err(PowerShellRenderError::new(
                    PowerShellRenderErrorCode::MissingEachItem,
                    None,
                ));
            };
            output.push_str(&serialize_single_quoted_literal(value));
        }
    }
    Ok(())
}

/// CMD 渲染阶段可稳定分类且不回显参数值的错误原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CmdRenderErrorCode {
    /// AST 引用的参数 key 不存在于规范化结果。
    MissingParameter,
    /// `if` 节点没有读取到 Boolean 值。
    InvalidIfValue,
    /// `each` 节点没有读取到 Folders 值。
    InvalidEachValue,
    /// `this` 节点脱离当前 `each` 迭代项。
    MissingEachItem,
    /// Folders 等值出现在固定 `echo(` 行无法表达的位置。
    UnsupportedValueContext,
    /// 参数值包含 CMD 私有变量不能安全承载的 NUL、CR 或 LF。
    ForbiddenValueCharacter,
    /// 单个环境变量值达到 CMD 的 8191 UTF-16 单元限制。
    ValueTooLong,
    /// Batch 源行或值展开后的物理命令行达到 8191 UTF-16 单元限制。
    ExpandedLineTooLong,
    /// AST 不是唯一允许的一次解析 `echo(` 行结构。
    UnsafeTemplateLine,
}

impl CmdRenderErrorCode {
    /// 返回供 Planner 和测试稳定匹配的 camelCase 原因码。
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::MissingParameter => "missingParameter",
            Self::InvalidIfValue => "invalidIfValue",
            Self::InvalidEachValue => "invalidEachValue",
            Self::MissingEachItem => "missingEachItem",
            Self::UnsupportedValueContext => "unsupportedValueContext",
            Self::ForbiddenValueCharacter => "forbiddenCmdValueCharacter",
            Self::ValueTooLong => "cmdValueTooLong",
            Self::ExpandedLineTooLong => "cmdExpandedLineTooLong",
            Self::UnsafeTemplateLine => "unsafeCmdTemplateLine",
        }
    }

    /// 返回当前错误是否由用户 Parameter Value 触发。
    pub(crate) const fn is_validation_error(self) -> bool {
        matches!(
            self,
            Self::ForbiddenValueCharacter | Self::ValueTooLong | Self::ExpandedLineTooLong
        )
    }

    /// 返回当前错误是否代表固定 CMD 模板越过行级 allowlist。
    pub(crate) const fn is_template_error(self) -> bool {
        matches!(self, Self::UnsafeTemplateLine)
    }
}

/// CMD 渲染阶段的窄错误，不携带参数原值、模板正文或本机路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CmdRenderError {
    /// 稳定失败原因。
    pub(crate) code: CmdRenderErrorCode,
    /// 与失败相关的 Parameter key；纯模板错误为空。
    pub(crate) parameter_key: Option<String>,
}

impl CmdRenderError {
    /// 创建一个不回显原始值的 CMD 渲染错误。
    fn new(code: CmdRenderErrorCode, parameter_key: Option<&str>) -> Self {
        Self {
            code,
            parameter_key: parameter_key.map(ToOwned::to_owned),
        }
    }
}

/// 输出只含稳定原因码和可选参数 key 的 CMD 渲染错误。
impl Display for CmdRenderError {
    /// 格式化不包含参数内容的错误说明。
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.parameter_key {
            Some(key) => write!(formatter, "CMD 参数 {key} 渲染失败：{}", self.code.as_str()),
            None => write!(formatter, "CMD 模板渲染失败：{}", self.code.as_str()),
        }
    }
}

/// CMD 渲染错误没有需要向外暴露的底层错误来源。
impl Error for CmdRenderError {}

/// 已完成严格行级渲染的 CMD Preview、UTF-8 Artifact 与私有字面绑定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderedCmd {
    /// 供 Preview 展示的完整 CRLF CMD 源码；不包含任何非空用户参数值。
    pub(crate) script_text: String,
    /// UTF-8 无 BOM、供完整 Hash 与 Execution 使用的最终 `.cmd`。
    pub(crate) artifact: RenderedScript,
    /// 由确定名称绑定的非空参数字面值，进入 Canonical Spec 并在启动时覆盖继承环境。
    pub(crate) private_environment: BTreeMap<String, OsString>,
}

/// 通过严格 AST/物理行 allowlist 渲染 CMD；只允许一次解析的固定 `echo(` 行。
pub(crate) fn render_cmd(
    ast: &TemplateAst,
    parameters: &NormalizedParameters,
) -> Result<RenderedCmd, CmdRenderError> {
    validate_cmd_nodes(&ast.nodes, true)?;

    let mut state = CmdRenderState::new()?;
    render_cmd_nodes(&ast.nodes, parameters, None, &mut state)?;
    state.finish()?;
    let script_text = state.script;
    let private_environment = state.private_environment;
    let artifact = RenderedScript::cmd(&script_text);
    Ok(RenderedCmd {
        script_text,
        artifact,
        private_environment,
    })
}

/// CMD allowlist 中一条物理行的静态文本或唯一 Value 占位。
#[derive(Debug, Clone, PartialEq, Eq)]
enum CmdLinePart {
    /// Rust 固定模板提供的静态文本。
    Text(String),
    /// 由 Serializer 绑定的单个类型化值。
    Value,
}

/// 只验证 AST 物理行结构、不读取或渲染参数值的 allowlist 状态机。
#[derive(Default)]
struct CmdTemplateValidator {
    /// 当前尚未遇到换行的物理行片段。
    current_line: Vec<CmdLinePart>,
}

impl CmdTemplateValidator {
    /// 把模板静态文本按 CRLF、LF 或 CR 切成物理行，并验证每个已完成行。
    fn push_text(&mut self, value: &str) -> Result<(), CmdRenderError> {
        let mut characters = value.chars().peekable();
        while let Some(character) = characters.next() {
            if character == '\r' {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                self.finish_line()?;
            } else if character == '\n' {
                self.finish_line()?;
            } else {
                match self.current_line.last_mut() {
                    Some(CmdLinePart::Text(text)) => text.push(character),
                    _ => self
                        .current_line
                        .push(CmdLinePart::Text(character.to_string())),
                }
            }
        }
        Ok(())
    }

    /// 在当前物理行追加一个参数占位。
    fn push_value(&mut self) {
        self.current_line.push(CmdLinePart::Value);
    }

    /// 验证当前行只可能是 `echo(<value>` 或固定 ASCII `echo(enabled`。
    fn finish_line(&mut self) -> Result<(), CmdRenderError> {
        let valid = matches!(
            self.current_line.as_slice(),
            [CmdLinePart::Text(prefix), CmdLinePart::Value] if prefix == "echo("
        ) || matches!(
            self.current_line.as_slice(),
            [CmdLinePart::Text(line)] if line == "echo(enabled"
        );
        self.current_line.clear();
        if valid {
            Ok(())
        } else {
            Err(CmdRenderError::new(
                CmdRenderErrorCode::UnsafeTemplateLine,
                None,
            ))
        }
    }

    /// 要求控制块只能位于物理行边界，避免跨块拼接出新的 CMD 结构。
    fn require_line_boundary(&self) -> Result<(), CmdRenderError> {
        if self.current_line.is_empty() {
            Ok(())
        } else {
            Err(CmdRenderError::new(
                CmdRenderErrorCode::UnsafeTemplateLine,
                None,
            ))
        }
    }

    /// 完成顶层模板；允许最后一行没有换行，但不允许空白或任意其他命令。
    fn finish(mut self) -> Result<(), CmdRenderError> {
        if self.current_line.is_empty() {
            Ok(())
        } else {
            self.finish_line()
        }
    }
}

/// 递归验证顶层与 if/each 体均只包含完整、一次解析的 `echo(` 行。
fn validate_cmd_nodes(nodes: &[TemplateNode], top_level: bool) -> Result<(), CmdRenderError> {
    let mut validator = CmdTemplateValidator::default();
    for node in nodes {
        match node {
            TemplateNode::Text { value } => validator.push_text(value)?,
            TemplateNode::Value { .. } => validator.push_value(),
            TemplateNode::If { body, .. } | TemplateNode::Each { body, .. } => {
                validator.require_line_boundary()?;
                validate_cmd_nodes(body, false)?;
            }
        }
    }
    if top_level {
        validator.finish()
    } else {
        validator.require_line_boundary()
    }
}

/// 当前 each 项及其来源参数 key，供错误定位和确定绑定使用。
#[derive(Clone, Copy)]
struct CmdEachItem<'a> {
    /// Folders Parameter key。
    parameter_key: &'a str,
    /// 当前规范化目录文本。
    value: &'a str,
}

/// CMD 源码、私有环境和两个物理行长度的同步渲染状态。
struct CmdRenderState {
    /// 正在生成的 CRLF Batch 源码。
    script: String,
    /// 非空用户值的确定私有绑定。
    private_environment: BTreeMap<String, OsString>,
    /// 下一个私有绑定的稳定遍历序号。
    next_binding_index: usize,
    /// 当前 Batch 源物理行的 UTF-16 单元数。
    source_line_units: usize,
    /// 当前行把私有引用替换为字面值后的 UTF-16 单元数。
    expanded_line_units: usize,
}

impl CmdRenderState {
    /// 以确定 chcp 路径绑定引用和 delayed expansion 前导创建渲染状态。
    fn new() -> Result<Self, CmdRenderError> {
        let mut state = Self {
            script: String::new(),
            private_environment: BTreeMap::new(),
            next_binding_index: 0,
            source_line_units: 0,
            expanded_line_units: 0,
        };
        state.push_static(&format!(
            "@\"!{CMD_CHCP_ENVIRONMENT_NAME}!\" 65001 >nul\r\n@setlocal EnableExtensions EnableDelayedExpansion\r\n@echo off\r\n"
        ))?;
        Ok(state)
    }

    /// 追加 Rust 固定静态文本，统一换行为 CRLF 并持续检查两个物理行长度。
    fn push_static(&mut self, value: &str) -> Result<(), CmdRenderError> {
        let mut characters = value.chars().peekable();
        while let Some(character) = characters.next() {
            if character == '\r' {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                self.push_newline();
            } else if character == '\n' {
                self.push_newline();
            } else {
                self.script.push(character);
                let units = character.len_utf16();
                self.source_line_units = self.source_line_units.saturating_add(units);
                self.expanded_line_units = self.expanded_line_units.saturating_add(units);
                self.ensure_line_limit(None)?;
            }
        }
        Ok(())
    }

    /// 追加一个值：空值只保留已有 `echo(`，非空值生成确定私有变量引用。
    fn push_value(&mut self, value: String, parameter_key: &str) -> Result<(), CmdRenderError> {
        if value
            .chars()
            .any(|character| matches!(character, '\0' | '\r' | '\n'))
        {
            return Err(CmdRenderError::new(
                CmdRenderErrorCode::ForbiddenValueCharacter,
                Some(parameter_key),
            ));
        }
        let value_units = value.encode_utf16().count();
        if value_units >= CMD_MAX_UTF16_UNITS {
            return Err(CmdRenderError::new(
                CmdRenderErrorCode::ValueTooLong,
                Some(parameter_key),
            ));
        }
        if value.is_empty() {
            return Ok(());
        }

        let name = format!(
            "{CMD_VALUE_ENVIRONMENT_PREFIX}{:08}",
            self.next_binding_index
        );
        self.next_binding_index = self.next_binding_index.saturating_add(1);
        let reference = format!("!{name}!");
        self.script.push_str(&reference);
        self.source_line_units = self
            .source_line_units
            .saturating_add(reference.encode_utf16().count());
        self.expanded_line_units = self.expanded_line_units.saturating_add(value_units);
        self.ensure_line_limit(Some(parameter_key))?;
        self.private_environment.insert(name, OsString::from(value));
        Ok(())
    }

    /// 写入规范 CRLF，并开始统计下一条物理命令行。
    fn push_newline(&mut self) {
        self.script.push_str("\r\n");
        self.source_line_units = 0;
        self.expanded_line_units = 0;
    }

    /// 达到 8191 UTF-16 单元即稳定拒绝，不把边界值交给 CMD 截断。
    fn ensure_line_limit(&self, parameter_key: Option<&str>) -> Result<(), CmdRenderError> {
        if self.source_line_units >= CMD_MAX_UTF16_UNITS
            || self.expanded_line_units >= CMD_MAX_UTF16_UNITS
        {
            return Err(CmdRenderError::new(
                CmdRenderErrorCode::ExpandedLineTooLong,
                parameter_key,
            ));
        }
        Ok(())
    }

    /// 完成脚本文本；allowlist 已保证模板不会留下半条不安全命令。
    fn finish(&self) -> Result<(), CmdRenderError> {
        self.ensure_line_limit(None)?;
        Ok(())
    }
}

/// 按 AST 顺序渲染固定 CMD 行；控制流只在 Rust 中决定是否复制已验证行。
fn render_cmd_nodes(
    nodes: &[TemplateNode],
    parameters: &NormalizedParameters,
    each_item: Option<CmdEachItem<'_>>,
    state: &mut CmdRenderState,
) -> Result<(), CmdRenderError> {
    for node in nodes {
        match node {
            TemplateNode::Text { value } => state.push_static(value)?,
            TemplateNode::Value { source } => {
                let (value, key) = cmd_value(source, parameters, each_item.as_ref())?;
                state.push_value(value, key)?;
            }
            TemplateNode::If { key, body } => match parameters.get(key) {
                Some(NormalizedParameterValue::Boolean(true)) => {
                    render_cmd_nodes(body, parameters, each_item, state)?;
                }
                Some(NormalizedParameterValue::Boolean(false)) | None => {}
                Some(_) => {
                    return Err(CmdRenderError::new(
                        CmdRenderErrorCode::InvalidIfValue,
                        Some(key),
                    ));
                }
            },
            TemplateNode::Each { key, body } => match parameters.get(key) {
                Some(NormalizedParameterValue::Folders(values)) => {
                    for value in values {
                        render_cmd_nodes(
                            body,
                            parameters,
                            Some(CmdEachItem {
                                parameter_key: key,
                                value,
                            }),
                            state,
                        )?;
                    }
                }
                None => {}
                Some(_) => {
                    return Err(CmdRenderError::new(
                        CmdRenderErrorCode::InvalidEachValue,
                        Some(key),
                    ));
                }
            },
        }
    }
    Ok(())
}

/// 把一个 AST Value 解析为 CMD 私有绑定文本和稳定错误定位 key。
fn cmd_value<'a>(
    source: &TemplateValueSource,
    parameters: &'a NormalizedParameters,
    each_item: Option<&'a CmdEachItem<'a>>,
) -> Result<(String, &'a str), CmdRenderError> {
    match source {
        TemplateValueSource::Parameter { key } => {
            let Some(entry) = parameters.entries.iter().find(|entry| entry.key == *key) else {
                return Err(CmdRenderError::new(
                    CmdRenderErrorCode::MissingParameter,
                    Some(key),
                ));
            };
            let value = match &entry.value {
                None => String::new(),
                Some(NormalizedParameterValue::Text(value))
                | Some(NormalizedParameterValue::Select(value))
                | Some(NormalizedParameterValue::Folder(value)) => value.clone(),
                Some(NormalizedParameterValue::Number(value)) => {
                    if *value == 0.0 {
                        "0".to_owned()
                    } else {
                        value.to_string()
                    }
                }
                Some(NormalizedParameterValue::Boolean(value)) => value.to_string(),
                Some(NormalizedParameterValue::Folders(_)) => {
                    return Err(CmdRenderError::new(
                        CmdRenderErrorCode::UnsupportedValueContext,
                        Some(key),
                    ));
                }
            };
            Ok((value, entry.key.as_str()))
        }
        TemplateValueSource::EachItem => {
            let Some(item) = each_item else {
                return Err(CmdRenderError::new(
                    CmdRenderErrorCode::MissingEachItem,
                    None,
                ));
            };
            Ok((item.value.to_owned(), item.parameter_key))
        }
    }
}

#[cfg(test)]
mod tests {
    //! PowerShell 字面量与 CMD 私有绑定、行级 allowlist 和长度边界测试。

    use std::ffi::{OsStr, OsString};

    use super::{
        render_cmd, render_windows_powershell, serialize_single_quoted_literal, CmdRenderErrorCode,
        CmdRenderState, CMD_MAX_UTF16_UNITS,
    };
    use crate::execution::parameter::{
        NormalizedParameter, NormalizedParameterValue, NormalizedParameters,
    };
    use crate::execution::template::{TemplateAst, TemplateNode, TemplateValueSource};

    /// 创建覆盖直接值、条件和多路径循环的已校验 AST。
    fn template_ast() -> TemplateAst {
        TemplateAst {
            nodes: vec![
                TemplateNode::Text {
                    value: "Write-Output ".to_owned(),
                },
                TemplateNode::Value {
                    source: TemplateValueSource::Parameter {
                        key: "text".to_owned(),
                    },
                },
                TemplateNode::Text {
                    value: "\n".to_owned(),
                },
                TemplateNode::If {
                    key: "enabled".to_owned(),
                    body: vec![TemplateNode::Text {
                        value: "Write-Output 'enabled'\n".to_owned(),
                    }],
                },
                TemplateNode::Each {
                    key: "folders".to_owned(),
                    body: vec![
                        TemplateNode::Text {
                            value: "Write-Output ".to_owned(),
                        },
                        TemplateNode::Value {
                            source: TemplateValueSource::EachItem,
                        },
                        TemplateNode::Text {
                            value: "\n".to_owned(),
                        },
                    ],
                },
            ],
        }
    }

    /// 创建覆盖中文、空格、单引号和多路径的规范化参数。
    fn normalized_parameters(enabled: bool) -> NormalizedParameters {
        NormalizedParameters {
            entries: vec![
                NormalizedParameter {
                    key: "text".to_owned(),
                    value: Some(NormalizedParameterValue::Text(
                        "中文 空格 user's value".to_owned(),
                    )),
                },
                NormalizedParameter {
                    key: "enabled".to_owned(),
                    value: Some(NormalizedParameterValue::Boolean(enabled)),
                },
                NormalizedParameter {
                    key: "folders".to_owned(),
                    value: Some(NormalizedParameterValue::Folders(vec![
                        r"C:\目录 一".to_owned(),
                        r"D:\folder's two".to_owned(),
                    ])),
                },
            ],
        }
    }

    /// 创建一条固定 `echo(` 值行，供 CMD 边界测试复用。
    fn cmd_value_ast(prefix: &str) -> TemplateAst {
        TemplateAst {
            nodes: vec![
                TemplateNode::Text {
                    value: prefix.to_owned(),
                },
                TemplateNode::Value {
                    source: TemplateValueSource::Parameter {
                        key: "text".to_owned(),
                    },
                },
                TemplateNode::Text {
                    value: "\n".to_owned(),
                },
            ],
        }
    }

    /// 创建只含一个 Text 的规范化参数。
    fn cmd_text_parameter(value: &str) -> NormalizedParameters {
        NormalizedParameters {
            entries: vec![NormalizedParameter {
                key: "text".to_owned(),
                value: Some(NormalizedParameterValue::Text(value.to_owned())),
            }],
        }
    }

    /// 验证 PowerShell 单引号字面量不会把输入中的单引号变成语法边界。
    #[test]
    fn doubles_single_quotes_inside_literal() {
        assert_eq!(
            serialize_single_quoted_literal("中文 user's path"),
            "'中文 user''s path'"
        );
    }

    /// 验证中文、空格、多路径和启用的 if/each 生成完全确定的 BOM 脚本。
    #[test]
    fn renders_unicode_if_and_each_deterministically() {
        let rendered = render_windows_powershell(&template_ast(), &normalized_parameters(true))
            .expect("已校验 AST 与参数应渲染");

        assert_eq!(
            rendered.script_text,
            "Write-Output '中文 空格 user''s value'\nWrite-Output 'enabled'\nWrite-Output 'C:\\目录 一'\nWrite-Output 'D:\\folder''s two'\n"
        );
        assert_eq!(&rendered.artifact.bytes()[..3], &[0xEF, 0xBB, 0xBF]);
        assert_eq!(
            &rendered.artifact.bytes()[3..],
            rendered.script_text.as_bytes()
        );
    }

    /// 验证 false 条件不输出固定块，但不会影响 each 的顺序与字面量语义。
    #[test]
    fn omits_false_if_without_changing_each_order() {
        let rendered = render_windows_powershell(&template_ast(), &normalized_parameters(false))
            .expect("false 条件仍应渲染");

        assert!(!rendered.script_text.contains("enabled"));
        assert!(rendered
            .script_text
            .ends_with("Write-Output 'C:\\目录 一'\nWrite-Output 'D:\\folder''s two'\n"));
    }

    /// 验证所有 CMD 元字符和多语言文本只进入私有环境，源码只保留一次解析变量引用。
    #[test]
    fn binds_cmd_unicode_and_metacharacters_without_putting_values_in_source() {
        let value = "中文 日本語 😀 space ' \" & % ^ ! ( ) < > | \\\\ CALL cmd /C for /f";
        let rendered = render_cmd(&cmd_value_ast("echo("), &cmd_text_parameter(value))
            .expect("安全值应通过私有环境绑定");

        assert_eq!(
            rendered.script_text,
            "@\"!CMDBOX_INTERNAL_CHCP!\" 65001 >nul\r\n@setlocal EnableExtensions EnableDelayedExpansion\r\n@echo off\r\necho(!CMDBOX_INTERNAL_VALUE_00000000!\r\n"
        );
        assert!(!rendered.script_text.contains(value));
        assert_eq!(
            rendered
                .private_environment
                .get("CMDBOX_INTERNAL_VALUE_00000000")
                .map(OsString::as_os_str),
            Some(OsStr::new(value))
        );
        assert_eq!(rendered.artifact.bytes(), rendered.script_text.as_bytes());
    }

    /// 验证空值静态渲染为 `echo(`，不创建会被 Windows 删除的空环境变量。
    #[test]
    fn renders_empty_cmd_value_without_environment_binding() {
        let rendered =
            render_cmd(&cmd_value_ast("echo("), &cmd_text_parameter("")).expect("空值应静态渲染");

        assert!(rendered.script_text.ends_with("echo(\r\n"));
        assert!(rendered.private_environment.is_empty());
    }

    /// 验证 NUL、CR 与 LF 都在进入环境块前按相同稳定原因拒绝。
    #[test]
    fn rejects_cmd_value_line_breaks_and_nul() {
        for value in ["nul\0value", "carriage\rreturn", "line\nfeed"] {
            let error = render_cmd(&cmd_value_ast("echo("), &cmd_text_parameter(value))
                .expect_err("控制字符不得进入 CMD 私有环境");
            assert_eq!(error.code, CmdRenderErrorCode::ForbiddenValueCharacter);
            assert_eq!(error.parameter_key.as_deref(), Some("text"));
            assert!(!error.to_string().contains(value));
        }
    }

    /// 验证单值以及带 `echo(` 前缀的物理展开达到 8191 UTF-16 单元时稳定拒绝。
    #[test]
    fn rejects_cmd_utf16_limits_at_boundary() {
        let mut value_boundary = CmdRenderState::new().expect("固定前导应有效");
        value_boundary
            .push_value("a".repeat(CMD_MAX_UTF16_UNITS - 1), "text")
            .expect("单变量 8190 UTF-16 单元本身应接受");
        let physical_boundary_value =
            "a".repeat(CMD_MAX_UTF16_UNITS - 1 - "echo(".encode_utf16().count());
        render_cmd(
            &cmd_value_ast("echo("),
            &cmd_text_parameter(&physical_boundary_value),
        )
        .expect("展开后物理行 8190 UTF-16 单元应接受");

        let value_error = render_cmd(
            &cmd_value_ast("echo("),
            &cmd_text_parameter(&"a".repeat(CMD_MAX_UTF16_UNITS)),
        )
        .expect_err("单值达到 8191 应拒绝");
        assert_eq!(value_error.code, CmdRenderErrorCode::ValueTooLong);

        let expanded_error = render_cmd(
            &cmd_value_ast("echo("),
            &cmd_text_parameter(&"😀".repeat((CMD_MAX_UTF16_UNITS - 5).div_ceil(2))),
        )
        .expect_err("物理展开达到 8191 应拒绝");
        assert_eq!(expanded_error.code, CmdRenderErrorCode::ExpandedLineTooLong);
    }

    /// 验证 CALL、嵌套 cmd 与 `for /f` 插值行均不在严格 AST allowlist 中。
    #[test]
    fn rejects_secondary_cmd_parse_contexts_structurally() {
        for prefix in [
            "call echo(",
            "CALL echo(",
            "cmd /C echo(",
            "CmD /k echo(",
            "for /f ('echo(",
            "FOR /F ('echo(",
        ] {
            let error = render_cmd(&cmd_value_ast(prefix), &cmd_text_parameter("safe"))
                .expect_err("二次解析上下文不得包含值插值");
            assert_eq!(error.code, CmdRenderErrorCode::UnsafeTemplateLine);
            assert_eq!(error.parameter_key, None);
        }

        let split_across_ast = TemplateAst {
            nodes: vec![
                TemplateNode::Text {
                    value: "CA".to_owned(),
                },
                TemplateNode::Text {
                    value: "LL echo(".to_owned(),
                },
                TemplateNode::Value {
                    source: TemplateValueSource::Parameter {
                        key: "text".to_owned(),
                    },
                },
                TemplateNode::Text {
                    value: " & echo(extra\n".to_owned(),
                },
            ],
        };
        let error = render_cmd(&split_across_ast, &cmd_text_parameter("safe"))
            .expect_err("拆分 AST 节点不得绕过完整物理行 allowlist");
        assert_eq!(error.code, CmdRenderErrorCode::UnsafeTemplateLine);

        let nested_control = TemplateAst {
            nodes: vec![TemplateNode::If {
                key: "enabled".to_owned(),
                body: cmd_value_ast("cmd /C echo(").nodes,
            }],
        };
        let parameters = NormalizedParameters {
            entries: vec![
                NormalizedParameter {
                    key: "enabled".to_owned(),
                    value: Some(NormalizedParameterValue::Boolean(true)),
                },
                NormalizedParameter {
                    key: "text".to_owned(),
                    value: Some(NormalizedParameterValue::Text("safe".to_owned())),
                },
            ],
        };
        let error =
            render_cmd(&nested_control, &parameters).expect_err("控制块内的二次解析行也必须拒绝");
        assert_eq!(error.code, CmdRenderErrorCode::UnsafeTemplateLine);
    }

    /// 验证 CMD 的 if/each 在 Rust 中展开，输出顺序与条件均保持确定。
    #[test]
    fn renders_cmd_if_and_each_in_rust() {
        let ast = TemplateAst {
            nodes: vec![
                TemplateNode::If {
                    key: "enabled".to_owned(),
                    body: vec![TemplateNode::Text {
                        value: "echo(enabled\n".to_owned(),
                    }],
                },
                TemplateNode::Each {
                    key: "folders".to_owned(),
                    body: vec![
                        TemplateNode::Text {
                            value: "echo(".to_owned(),
                        },
                        TemplateNode::Value {
                            source: TemplateValueSource::EachItem,
                        },
                        TemplateNode::Text {
                            value: "\n".to_owned(),
                        },
                    ],
                },
            ],
        };
        let parameters = NormalizedParameters {
            entries: vec![
                NormalizedParameter {
                    key: "enabled".to_owned(),
                    value: Some(NormalizedParameterValue::Boolean(true)),
                },
                NormalizedParameter {
                    key: "folders".to_owned(),
                    value: Some(NormalizedParameterValue::Folders(vec![
                        r"C:\目录 一".to_owned(),
                        r"D:\日本語 😀 & % !".to_owned(),
                    ])),
                },
            ],
        };

        let rendered = render_cmd(&ast, &parameters).expect("if/each 应由 Rust 确定展开");

        assert!(rendered.script_text.contains("echo(enabled\r\n"));
        assert!(rendered.script_text.ends_with(
            "echo(!CMDBOX_INTERNAL_VALUE_00000000!\r\necho(!CMDBOX_INTERNAL_VALUE_00000001!\r\n"
        ));
        assert_eq!(rendered.private_environment.len(), 2);
    }
}
