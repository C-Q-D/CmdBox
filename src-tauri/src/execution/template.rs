//! CmdBox 受限模板的确定性 Parser、Definition 引用校验与 AST。
//!
//! 模板只承认参数 value、Boolean `if` 和 Folders `each` 三种语义指令；Shell
//! 字面量序列化属于后续模块。本 Parser 不执行表达式、helper、raw、函数或 Shell 逻辑，
//! 并禁止控制块嵌套，使相同模板与 Definition 始终产生相同 AST。

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};

use super::parameter::{is_valid_parameter_key, ParameterDefinition, ParameterKind};

/// 一个已经完成 Definition 引用校验的受限模板 AST。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateAst {
    /// 按模板源码顺序保存的顶层节点。
    pub nodes: Vec<TemplateNode>,
}

/// 模板的字面文本以及三种受限语义节点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TemplateNode {
    /// 不经过解释、原样保留的 Command Block 静态模板文本。
    Text {
        /// 当前字面文本片段。
        value: String,
    },
    /// 一个参数值或当前 each 项的占位节点。
    Value {
        /// 后续 Renderer 应读取的类型化值来源。
        source: TemplateValueSource,
    },
    /// 仅由 Boolean 参数控制的无嵌套条件节点。
    If {
        /// 已确认属于 Boolean Definition 的参数 key。
        key: String,
        /// 只包含 Text/Value 的条件体。
        body: Vec<TemplateNode>,
    },
    /// 仅迭代 Folders 参数的无嵌套循环节点。
    Each {
        /// 已确认属于 Folders Definition 的参数 key。
        key: String,
        /// 只包含 Text/Value 的循环体；`EachItem` 只会出现在这里。
        body: Vec<TemplateNode>,
    },
}

/// Value 节点读取的结构化值来源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TemplateValueSource {
    /// 按 key 读取一个已经规范化的 Parameter Value。
    Parameter {
        /// 已在当前 Definition 集合中解析成功的 key。
        key: String,
    },
    /// 读取当前 `each` 迭代项；Parser 保证它不会出现在 each 之外。
    EachItem,
}

/// 受限模板 Parser 和 Validator 的稳定错误码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TemplateErrorCode {
    /// Definition key 非法或保留字被占用。
    InvalidDefinition,
    /// Definition 集合重复声明同一 key。
    DuplicateDefinitionKey,
    /// 标签不是允许的 value/if/each/this 语法。
    InvalidSyntax,
    /// `{{` 没有对应的 `}}`。
    UnclosedTag,
    /// 模板文本出现没有对应开放标签的 `}}`。
    UnexpectedClosingDelimiter,
    /// if/each 控制块内部再次打开控制块。
    NestedBlock,
    /// 没有开放控制块却出现关闭标签。
    UnexpectedClosingBlock,
    /// 关闭标签与当前开放块类型不一致。
    MismatchedClosingBlock,
    /// 模板结束时仍有未关闭控制块。
    UnclosedBlock,
    /// value/if/each 引用了未声明的变量。
    UndefinedVariable,
    /// if 引用的 Definition 不是 Boolean。
    InvalidIfParameter,
    /// each 引用的 Definition 不是 Folders。
    InvalidEachParameter,
    /// `this` 出现在 each 之外。
    ThisOutsideEach,
}

impl TemplateErrorCode {
    /// 返回跨 IPC 和测试可稳定比较的 camelCase 错误码。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidDefinition => "invalidDefinition",
            Self::DuplicateDefinitionKey => "duplicateDefinitionKey",
            Self::InvalidSyntax => "invalidSyntax",
            Self::UnclosedTag => "unclosedTag",
            Self::UnexpectedClosingDelimiter => "unexpectedClosingDelimiter",
            Self::NestedBlock => "nestedBlock",
            Self::UnexpectedClosingBlock => "unexpectedClosingBlock",
            Self::MismatchedClosingBlock => "mismatchedClosingBlock",
            Self::UnclosedBlock => "unclosedBlock",
            Self::UndefinedVariable => "undefinedVariable",
            Self::InvalidIfParameter => "invalidIfParameter",
            Self::InvalidEachParameter => "invalidEachParameter",
            Self::ThisOutsideEach => "thisOutsideEach",
        }
    }
}

/// 不回显模板正文的受限模板错误。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateError {
    /// 可稳定匹配的解析或引用错误原因。
    pub code: TemplateErrorCode,
    /// 与错误直接相关的参数 key；纯语法错误为 `None`。
    pub key: Option<String>,
}

impl TemplateError {
    /// 创建一个不包含模板正文的稳定错误。
    fn new(code: TemplateErrorCode, key: Option<&str>) -> Self {
        Self {
            code,
            key: key.map(ToOwned::to_owned),
        }
    }
}

/// 输出不包含模板正文或参数值的模板错误说明。
impl Display for TemplateError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.key {
            Some(key) => write!(formatter, "模板参数 {key} 校验失败：{}", self.code.as_str()),
            None => write!(formatter, "模板校验失败：{}", self.code.as_str()),
        }
    }
}

/// Template Parser 失败没有需要向上暴露的底层错误来源。
impl Error for TemplateError {}

/// 当前无嵌套控制块的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    /// Boolean 条件块。
    If,
    /// Folders 循环块。
    Each,
}

/// Parser 尚未读到关闭标签的唯一开放控制块。
struct OpenBlock {
    /// 开放块类型。
    kind: BlockKind,
    /// 控制块引用的 Definition key。
    key: String,
    /// 已解析且只含 Text/Value 的块体。
    body: Vec<TemplateNode>,
}

/// 把受限模板解析为确定 AST，并校验所有变量与当前 Definition 的类型关系。
pub fn parse_template(
    template: &str,
    definitions: &[ParameterDefinition],
) -> Result<TemplateAst, TemplateError> {
    let parameter_kinds = collect_parameter_kinds(definitions)?;
    let mut nodes = Vec::new();
    let mut open_block: Option<OpenBlock> = None;
    let mut cursor = 0;

    while cursor < template.len() {
        let remaining = &template[cursor..];
        let next_open = remaining.find("{{");
        let next_close = remaining.find("}}");

        if next_close.is_some_and(|closing| next_open.is_none_or(|opening| closing < opening)) {
            return Err(TemplateError::new(
                TemplateErrorCode::UnexpectedClosingDelimiter,
                None,
            ));
        }

        let Some(opening_offset) = next_open else {
            append_text(&mut nodes, &mut open_block, remaining);
            cursor = template.len();
            continue;
        };

        append_text(&mut nodes, &mut open_block, &remaining[..opening_offset]);
        let tag_start = cursor + opening_offset + 2;
        let Some(closing_offset) = template[tag_start..].find("}}") else {
            return Err(TemplateError::new(TemplateErrorCode::UnclosedTag, None));
        };
        let tag_end = tag_start + closing_offset;
        let tag = &template[tag_start..tag_end];
        handle_tag(tag, &parameter_kinds, &mut nodes, &mut open_block)?;
        cursor = tag_end + 2;
    }

    if let Some(block) = open_block {
        return Err(TemplateError::new(
            TemplateErrorCode::UnclosedBlock,
            Some(&block.key),
        ));
    }

    Ok(TemplateAst { nodes })
}

/// 收集 Definition key 与类型，并拒绝 Parser 无法无歧义引用的 Definition。
fn collect_parameter_kinds(
    definitions: &[ParameterDefinition],
) -> Result<BTreeMap<&str, ParameterKind>, TemplateError> {
    let mut kinds = BTreeMap::new();
    for definition in definitions {
        let key = definition.key();
        if !is_valid_parameter_key(key) {
            return Err(TemplateError::new(
                TemplateErrorCode::InvalidDefinition,
                Some(key),
            ));
        }
        if kinds.insert(key, definition.kind()).is_some() {
            return Err(TemplateError::new(
                TemplateErrorCode::DuplicateDefinitionKey,
                Some(key),
            ));
        }
    }
    Ok(kinds)
}

/// 解析并校验一个不包含花括号的标签正文。
fn handle_tag(
    tag: &str,
    parameter_kinds: &BTreeMap<&str, ParameterKind>,
    nodes: &mut Vec<TemplateNode>,
    open_block: &mut Option<OpenBlock>,
) -> Result<(), TemplateError> {
    if let Some(key) = tag.strip_prefix("#if ") {
        return open_control_block(
            BlockKind::If,
            key,
            parameter_kinds,
            open_block,
            ParameterKind::Boolean,
            TemplateErrorCode::InvalidIfParameter,
        );
    }
    if let Some(key) = tag.strip_prefix("#each ") {
        return open_control_block(
            BlockKind::Each,
            key,
            parameter_kinds,
            open_block,
            ParameterKind::Folders,
            TemplateErrorCode::InvalidEachParameter,
        );
    }
    if tag == "/if" {
        return close_control_block(BlockKind::If, nodes, open_block);
    }
    if tag == "/each" {
        return close_control_block(BlockKind::Each, nodes, open_block);
    }
    if tag == "this" {
        if !matches!(
            open_block.as_ref().map(|block| block.kind),
            Some(BlockKind::Each)
        ) {
            return Err(TemplateError::new(
                TemplateErrorCode::ThisOutsideEach,
                Some("this"),
            ));
        }
        append_node(
            nodes,
            open_block,
            TemplateNode::Value {
                source: TemplateValueSource::EachItem,
            },
        );
        return Ok(());
    }
    if !is_valid_parameter_key(tag) {
        return Err(TemplateError::new(TemplateErrorCode::InvalidSyntax, None));
    }
    if !parameter_kinds.contains_key(tag) {
        return Err(TemplateError::new(
            TemplateErrorCode::UndefinedVariable,
            Some(tag),
        ));
    }

    append_node(
        nodes,
        open_block,
        TemplateNode::Value {
            source: TemplateValueSource::Parameter {
                key: tag.to_owned(),
            },
        },
    );
    Ok(())
}

/// 打开一个顶层 if/each 块，并校验其参数类型；任何控制块嵌套都拒绝。
fn open_control_block(
    kind: BlockKind,
    key: &str,
    parameter_kinds: &BTreeMap<&str, ParameterKind>,
    open_block: &mut Option<OpenBlock>,
    expected_kind: ParameterKind,
    wrong_kind_code: TemplateErrorCode,
) -> Result<(), TemplateError> {
    if open_block.is_some() {
        return Err(TemplateError::new(
            TemplateErrorCode::NestedBlock,
            Some(key),
        ));
    }
    if !is_valid_parameter_key(key) {
        return Err(TemplateError::new(TemplateErrorCode::InvalidSyntax, None));
    }
    let Some(actual_kind) = parameter_kinds.get(key) else {
        return Err(TemplateError::new(
            TemplateErrorCode::UndefinedVariable,
            Some(key),
        ));
    };
    if *actual_kind != expected_kind {
        return Err(TemplateError::new(wrong_kind_code, Some(key)));
    }

    *open_block = Some(OpenBlock {
        kind,
        key: key.to_owned(),
        body: Vec::new(),
    });
    Ok(())
}

/// 关闭当前顶层控制块，并把完成的 If/Each 节点追加到 AST。
fn close_control_block(
    expected_kind: BlockKind,
    nodes: &mut Vec<TemplateNode>,
    open_block: &mut Option<OpenBlock>,
) -> Result<(), TemplateError> {
    let Some(block) = open_block.take() else {
        return Err(TemplateError::new(
            TemplateErrorCode::UnexpectedClosingBlock,
            None,
        ));
    };
    if block.kind != expected_kind {
        let key = block.key.clone();
        *open_block = Some(block);
        return Err(TemplateError::new(
            TemplateErrorCode::MismatchedClosingBlock,
            Some(&key),
        ));
    }

    let node = match block.kind {
        BlockKind::If => TemplateNode::If {
            key: block.key,
            body: block.body,
        },
        BlockKind::Each => TemplateNode::Each {
            key: block.key,
            body: block.body,
        },
    };
    nodes.push(node);
    Ok(())
}

/// 把非空字面文本追加到当前开放块或顶层节点序列。
fn append_text(nodes: &mut Vec<TemplateNode>, open_block: &mut Option<OpenBlock>, value: &str) {
    if value.is_empty() {
        return;
    }
    append_node(
        nodes,
        open_block,
        TemplateNode::Text {
            value: value.to_owned(),
        },
    );
}

/// 按当前 Parser 位置把一个合法的 Text/Value 节点追加到块体或顶层。
fn append_node(
    nodes: &mut Vec<TemplateNode>,
    open_block: &mut Option<OpenBlock>,
    node: TemplateNode,
) {
    match open_block {
        Some(block) => block.body.push(node),
        None => nodes.push(node),
    }
}

#[cfg(test)]
mod tests {
    //! value/if/each AST 与所有被禁止语法边界的直接单元测试。

    use super::{
        parse_template, TemplateAst, TemplateErrorCode, TemplateNode, TemplateValueSource,
    };
    use crate::execution::parameter::{
        BooleanParameterDefinition, FoldersParameterDefinition, ParameterBase, ParameterDefinition,
        TextParameterDefinition,
    };

    /// 创建模板测试所需的 Parameter Definition 共享元数据。
    fn base(key: &str) -> ParameterBase {
        ParameterBase {
            key: key.to_owned(),
            label: format!("{key} 参数"),
            description: None,
            required: true,
            remember: false,
        }
    }

    /// 创建包含 Text、Boolean 与 Folders 的模板类型环境。
    fn definitions() -> Vec<ParameterDefinition> {
        vec![
            ParameterDefinition::Text(TextParameterDefinition {
                base: base("text"),
                default_value: None,
                min_length: None,
                max_length: None,
                placeholder: None,
            }),
            ParameterDefinition::Boolean(BooleanParameterDefinition {
                base: base("enabled"),
                default_value: false,
            }),
            ParameterDefinition::Folders(FoldersParameterDefinition {
                base: base("folders"),
                must_exist: false,
                min_items: Some(1),
                max_items: None,
                default_value: None,
            }),
        ]
    }

    /// 验证 value、Boolean if、Folders each 与 each 内 this 产生确定 AST。
    #[test]
    fn parses_value_if_each_and_this_into_stable_ast() {
        let template = "前缀 {{text}}\n{{#if enabled}}已启用 {{text}}{{/if}}\n{{#each folders}}目录={{this}} {{text}}\n{{/each}}尾部";

        let ast = parse_template(template, &definitions()).expect("受限模板应解析");

        assert_eq!(
            ast,
            TemplateAst {
                nodes: vec![
                    TemplateNode::Text {
                        value: "前缀 ".to_owned(),
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
                        body: vec![
                            TemplateNode::Text {
                                value: "已启用 ".to_owned(),
                            },
                            TemplateNode::Value {
                                source: TemplateValueSource::Parameter {
                                    key: "text".to_owned(),
                                },
                            },
                        ],
                    },
                    TemplateNode::Text {
                        value: "\n".to_owned(),
                    },
                    TemplateNode::Each {
                        key: "folders".to_owned(),
                        body: vec![
                            TemplateNode::Text {
                                value: "目录=".to_owned(),
                            },
                            TemplateNode::Value {
                                source: TemplateValueSource::EachItem,
                            },
                            TemplateNode::Text {
                                value: " ".to_owned(),
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
                    },
                    TemplateNode::Text {
                        value: "尾部".to_owned(),
                    },
                ],
            }
        );
    }

    /// 验证 value、if 和 each 都拒绝未定义变量。
    #[test]
    fn rejects_undefined_variables_in_every_directive() {
        for template in [
            "{{missing}}",
            "{{#if missing}}x{{/if}}",
            "{{#each missing}}{{this}}{{/each}}",
        ] {
            let error = parse_template(template, &definitions()).expect_err("未定义变量应拒绝");
            assert_eq!(error.code, TemplateErrorCode::UndefinedVariable);
            assert_eq!(error.key.as_deref(), Some("missing"));
        }
    }

    /// 验证 if 只接受 Boolean，each 只接受 Folders。
    #[test]
    fn rejects_wrong_if_and_each_parameter_kinds() {
        let if_error =
            parse_template("{{#if text}}x{{/if}}", &definitions()).expect_err("Text 不得控制 if");
        assert_eq!(if_error.code, TemplateErrorCode::InvalidIfParameter);

        let each_error = parse_template("{{#each text}}{{this}}{{/each}}", &definitions())
            .expect_err("Text 不得控制 each");
        assert_eq!(each_error.code, TemplateErrorCode::InvalidEachParameter);
    }

    /// 验证任何 if/each 控制块嵌套都被稳定拒绝。
    #[test]
    fn rejects_nested_control_blocks() {
        let template = "{{#each folders}}{{#if enabled}}{{this}}{{/if}}{{/each}}";

        let error = parse_template(template, &definitions()).expect_err("嵌套控制块应拒绝");

        assert_eq!(error.code, TemplateErrorCode::NestedBlock);
        assert_eq!(error.key.as_deref(), Some("enabled"));
    }

    /// 验证 this 在顶层或 if 中均不能获得错误的迭代语义。
    #[test]
    fn rejects_this_outside_each() {
        for template in ["{{this}}", "{{#if enabled}}{{this}}{{/if}}"] {
            let error = parse_template(template, &definitions()).expect_err("非法 this 应拒绝");
            assert_eq!(error.code, TemplateErrorCode::ThisOutsideEach);
            assert_eq!(error.key.as_deref(), Some("this"));
        }
    }

    /// 验证 raw、属性表达式、helper 参数和带空白的 value 标签均不进入 AST。
    #[test]
    fn rejects_raw_expressions_and_helpers() {
        for template in [
            "{{{text}}}",
            "{{text.value}}",
            "{{helper text}}",
            "{{ text }}",
            "{{& text}}",
        ] {
            let error = parse_template(template, &definitions()).expect_err("扩展语法应拒绝");
            assert_eq!(error.code, TemplateErrorCode::InvalidSyntax);
        }
    }

    /// 验证未闭合标签、错误关闭块与未闭合控制块都返回稳定语法错误。
    #[test]
    fn rejects_unbalanced_tags_and_blocks() {
        let cases = [
            ("{{text", TemplateErrorCode::UnclosedTag),
            ("text }}", TemplateErrorCode::UnexpectedClosingDelimiter),
            ("{{/if}}", TemplateErrorCode::UnexpectedClosingBlock),
            (
                "{{#if enabled}}x{{/each}}",
                TemplateErrorCode::MismatchedClosingBlock,
            ),
            ("{{#if enabled}}x", TemplateErrorCode::UnclosedBlock),
        ];

        for (template, expected_code) in cases {
            let error = parse_template(template, &definitions()).expect_err("不平衡模板应拒绝");
            assert_eq!(error.code, expected_code);
        }
    }
}
