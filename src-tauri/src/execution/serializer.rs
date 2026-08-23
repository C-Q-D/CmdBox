//! Windows PowerShell 受限模板的字面量序列化与确定性渲染。
//!
//! 本模块只消费已经由 Parameter Validator 规范化的值和已通过引用校验的 Template AST，
//! 将所有业务值编码为 PowerShell 字面量。它不读取文件、不解析任意表达式，也不启动进程；
//! 最终产物统一交给 `RenderedScript` 冻结为带 UTF-8 BOM 的完整字节。

use std::error::Error;
use std::fmt::{Display, Formatter};

use super::artifact::RenderedScript;
use super::parameter::{NormalizedParameterValue, NormalizedParameters};
use super::template::{TemplateAst, TemplateNode, TemplateValueSource};

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

#[cfg(test)]
mod tests {
    //! PowerShell 单引号、六类字面量与 if/each 确定性渲染测试。

    use super::{render_windows_powershell, serialize_single_quoted_literal};
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
}
