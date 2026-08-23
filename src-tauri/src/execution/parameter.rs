//! Command Block 类型化参数定义、结构化输入与确定性规范化。
//!
//! 本模块位于 Rust Core 信任边界内，只根据当前 Parameter Definition 校验一次请求的
//! 结构化值。它不拼接 Shell 文本；Folder/Folders 只做词法路径规范化，并在
//! `must_exist` 开启时读取目标根对象的目录元数据，不遍历目录内容。

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 一次 Preview 或 Execution 请求提交的结构化参数集合。
///
/// 使用 `BTreeMap` 让未知 key 的错误选择保持稳定；最终规范化顺序仍以 Definition 顺序为准。
pub type ParameterValues = BTreeMap<String, ParameterValue>;

/// 不依赖 `serde_json` 的完整 JSON 值语义，用于在 Rust Core 内继续执行类型化校验。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParameterValue {
    /// JSON `null`。
    Null,
    /// JSON Boolean。
    Boolean(bool),
    /// JSON Number；校验阶段还会拒绝非有限的 Rust 构造值。
    Number(f64),
    /// JSON String。
    Text(String),
    /// JSON Array。
    Array(Vec<ParameterValue>),
    /// JSON Object；当前六类参数均不接受对象，但保留它以返回带 key 的稳定类型错误。
    Object(BTreeMap<String, ParameterValue>),
}

/// 六类 Parameter Definition 共享的业务元数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterBase {
    /// 在一个 Command Block 内唯一且可被受限模板引用的稳定 key。
    pub key: String,
    /// Command Workspace 展示的参数名称。
    pub label: String,
    /// Command Workspace 可选展示的参数说明。
    pub description: Option<String>,
    /// 请求是否必须显式提交该 key；默认值不会替代 required 输入。
    pub required: bool,
    /// 后续持久化层是否允许记忆该参数；本模块不执行持久化。
    pub remember: bool,
}

/// Text 参数的长度和默认值约束。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextParameterDefinition {
    /// Text 参数共享元数据。
    #[serde(flatten)]
    pub base: ParameterBase,
    /// optional 参数未提交时使用的默认文本。
    pub default_value: Option<String>,
    /// 按 Unicode scalar value 计数的最小长度。
    pub min_length: Option<usize>,
    /// 按 Unicode scalar value 计数的最大长度。
    pub max_length: Option<usize>,
    /// 仅供 Command Workspace 展示的输入提示。
    pub placeholder: Option<String>,
}

/// Number 参数的有限值、范围和步长约束。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NumberParameterDefinition {
    /// Number 参数共享元数据。
    #[serde(flatten)]
    pub base: ParameterBase,
    /// optional 参数未提交时使用的默认数字。
    pub default_value: Option<f64>,
    /// 可接受的最小值，包含边界。
    pub min: Option<f64>,
    /// 可接受的最大值，包含边界。
    pub max: Option<f64>,
    /// 相对 `min`（没有 `min` 时相对零）的正有限步长。
    pub step: Option<f64>,
}

/// Boolean 参数及其明确默认值。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BooleanParameterDefinition {
    /// Boolean 参数共享元数据。
    #[serde(flatten)]
    pub base: ParameterBase,
    /// optional 参数未提交时使用的明确布尔默认值。
    pub default_value: bool,
}

/// Select 参数的固定选项集合。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectParameterDefinition {
    /// Select 参数共享元数据。
    #[serde(flatten)]
    pub base: ParameterBase,
    /// 可提交的固定字符串选项；比较区分大小写。
    pub options: Vec<String>,
    /// optional 参数未提交时使用的固定选项。
    pub default_value: Option<String>,
}

/// Folder 参数的绝对目录约束。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderParameterDefinition {
    /// Folder 参数共享元数据。
    #[serde(flatten)]
    pub base: ParameterBase,
    /// 是否要求规范化后的目标根对象当前存在且是目录。
    pub must_exist: bool,
    /// optional 参数未提交时使用的绝对目录文本。
    pub default_value: Option<String>,
}

/// Folders 参数的绝对目录数组和数量约束。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoldersParameterDefinition {
    /// Folders 参数共享元数据。
    #[serde(flatten)]
    pub base: ParameterBase,
    /// 是否要求每个规范化目标根对象当前存在且是目录。
    pub must_exist: bool,
    /// 可接受的最少目录项数量。
    pub min_items: Option<usize>,
    /// 可接受的最多目录项数量。
    pub max_items: Option<usize>,
    /// optional 参数未提交时使用的绝对目录文本数组。
    pub default_value: Option<Vec<String>>,
}

/// 当前 CMD-02 原子支持的六类 Parameter Definition。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ParameterDefinition {
    /// 单个文本值。
    Text(TextParameterDefinition),
    /// 单个有限数字值。
    Number(NumberParameterDefinition),
    /// 单个布尔值。
    Boolean(BooleanParameterDefinition),
    /// 固定选项中的单个字符串值。
    Select(SelectParameterDefinition),
    /// 单个绝对目录文本。
    Folder(FolderParameterDefinition),
    /// 多个绝对目录文本。
    Folders(FoldersParameterDefinition),
}

/// Template Validator 需要识别的参数语义类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParameterKind {
    /// Text 参数。
    Text,
    /// Number 参数。
    Number,
    /// Boolean 参数。
    Boolean,
    /// Select 参数。
    Select,
    /// Folder 参数。
    Folder,
    /// Folders 参数。
    Folders,
}

impl ParameterDefinition {
    /// 返回 Definition 的共享元数据。
    pub fn base(&self) -> &ParameterBase {
        match self {
            Self::Text(definition) => &definition.base,
            Self::Number(definition) => &definition.base,
            Self::Boolean(definition) => &definition.base,
            Self::Select(definition) => &definition.base,
            Self::Folder(definition) => &definition.base,
            Self::Folders(definition) => &definition.base,
        }
    }

    /// 返回受限模板和调用方使用的稳定参数 key。
    pub fn key(&self) -> &str {
        &self.base().key
    }

    /// 返回当前 Definition 的参数语义类别。
    pub fn kind(&self) -> ParameterKind {
        match self {
            Self::Text(_) => ParameterKind::Text,
            Self::Number(_) => ParameterKind::Number,
            Self::Boolean(_) => ParameterKind::Boolean,
            Self::Select(_) => ParameterKind::Select,
            Self::Folder(_) => ParameterKind::Folder,
            Self::Folders(_) => ParameterKind::Folders,
        }
    }

    /// 把 Definition 的类型化默认值转换为结构化输入语义，供同一校验路径处理。
    fn default_parameter_value(&self) -> Option<ParameterValue> {
        match self {
            Self::Text(definition) => definition
                .default_value
                .as_ref()
                .map(|value| ParameterValue::Text(value.clone())),
            Self::Number(definition) => definition.default_value.map(ParameterValue::Number),
            Self::Boolean(definition) => Some(ParameterValue::Boolean(definition.default_value)),
            Self::Select(definition) => definition
                .default_value
                .as_ref()
                .map(|value| ParameterValue::Text(value.clone())),
            Self::Folder(definition) => definition
                .default_value
                .as_ref()
                .map(|value| ParameterValue::Text(value.clone())),
            Self::Folders(definition) => definition.default_value.as_ref().map(|values| {
                ParameterValue::Array(values.iter().cloned().map(ParameterValue::Text).collect())
            }),
        }
    }
}

/// 通过当前 Definition 校验后的单个参数值。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum NormalizedParameterValue {
    /// 保留原始 Unicode 内容的 Text。
    Text(String),
    /// 已通过有限值、范围和 step 校验的 Number。
    Number(f64),
    /// Boolean。
    Boolean(bool),
    /// 已确认属于固定 options 的 Select。
    Select(String),
    /// 经过词法规范化的绝对 Folder 文本。
    Folder(String),
    /// 按请求顺序保存、逐项词法规范化的绝对 Folders 文本。
    Folders(Vec<String>),
}

/// 一个 Definition 对应的规范化结果；optional 且没有默认值时为 `None`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedParameter {
    /// 对应 Parameter Definition 的稳定 key。
    pub key: String,
    /// 已验证值，或 optional 缺省状态。
    pub value: Option<NormalizedParameterValue>,
}

/// 严格按 Parameter Definition 顺序排列的完整规范化参数集合。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedParameters {
    /// 每个 Definition 唯一对应的规范化项。
    pub entries: Vec<NormalizedParameter>,
}

impl NormalizedParameters {
    /// 按 key 返回规范化项；遍历顺序始终保持 Definition 顺序。
    pub fn get(&self, key: &str) -> Option<&NormalizedParameterValue> {
        self.entries
            .iter()
            .find(|entry| entry.key == key)
            .and_then(|entry| entry.value.as_ref())
    }
}

/// Parameter Validator 的稳定错误码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParameterErrorCode {
    /// Definition 自身的约束组合无效。
    InvalidDefinition,
    /// 一个 Definition 集合重复声明同一 key。
    DuplicateDefinitionKey,
    /// 请求包含当前 Definition 未声明的 key。
    UnknownKey,
    /// 请求缺少 required key。
    MissingRequired,
    /// required Text/Folders 显式提交了空值。
    EmptyRequired,
    /// JSON 值类别与 Definition 不一致。
    InvalidType,
    /// Text 短于 `min_length`。
    TextTooShort,
    /// Text 长于 `max_length`。
    TextTooLong,
    /// Number 不是有限值。
    NumberNotFinite,
    /// Number 小于 `min`。
    NumberBelowMinimum,
    /// Number 大于 `max`。
    NumberAboveMaximum,
    /// Number 不满足相对 step 基点的步长。
    NumberStepMismatch,
    /// Select 不属于固定 options。
    SelectOptionInvalid,
    /// Folder 文本为空或包含无法作为路径处理的字符。
    FolderPathInvalid,
    /// Folder 文本不是当前平台的绝对路径。
    FolderPathNotAbsolute,
    /// Folder 的 `..` 试图越过绝对路径根。
    FolderPathEscapesRoot,
    /// `must_exist` 目标根对象不存在。
    FolderNotFound,
    /// `must_exist` 目标根对象存在但不是目录。
    FolderNotDirectory,
    /// 无法读取 `must_exist` 目标根对象的元数据。
    FolderUnavailable,
    /// Folders 少于 `min_items`。
    FoldersTooFew,
    /// Folders 多于 `max_items`。
    FoldersTooMany,
    /// Folders 数组中至少一项不是 JSON String。
    FoldersItemInvalidType,
}

impl ParameterErrorCode {
    /// 返回跨 IPC 和测试可稳定比较的 camelCase 错误码。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidDefinition => "invalidDefinition",
            Self::DuplicateDefinitionKey => "duplicateDefinitionKey",
            Self::UnknownKey => "unknownKey",
            Self::MissingRequired => "missingRequired",
            Self::EmptyRequired => "emptyRequired",
            Self::InvalidType => "invalidType",
            Self::TextTooShort => "textTooShort",
            Self::TextTooLong => "textTooLong",
            Self::NumberNotFinite => "numberNotFinite",
            Self::NumberBelowMinimum => "numberBelowMinimum",
            Self::NumberAboveMaximum => "numberAboveMaximum",
            Self::NumberStepMismatch => "numberStepMismatch",
            Self::SelectOptionInvalid => "selectOptionInvalid",
            Self::FolderPathInvalid => "folderPathInvalid",
            Self::FolderPathNotAbsolute => "folderPathNotAbsolute",
            Self::FolderPathEscapesRoot => "folderPathEscapesRoot",
            Self::FolderNotFound => "folderNotFound",
            Self::FolderNotDirectory => "folderNotDirectory",
            Self::FolderUnavailable => "folderUnavailable",
            Self::FoldersTooFew => "foldersTooFew",
            Self::FoldersTooMany => "foldersTooMany",
            Self::FoldersItemInvalidType => "foldersItemInvalidType",
        }
    }
}

/// 仅包含参数 key 和稳定错误码的校验失败，不携带本机私有路径文本。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterValidationError {
    /// 失败的 Definition key，未知 key 时为请求中的 key。
    pub key: String,
    /// 可稳定匹配的错误原因。
    pub code: ParameterErrorCode,
}

impl ParameterValidationError {
    /// 创建一个不包含原始参数值的稳定校验错误。
    fn new(key: impl Into<String>, code: ParameterErrorCode) -> Self {
        Self {
            key: key.into(),
            code,
        }
    }
}

/// 输出不回显原始值或路径的参数校验说明。
impl Display for ParameterValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "参数 {} 校验失败：{}",
            self.key,
            self.code.as_str()
        )
    }
}

/// Parameter 校验失败没有需要向上暴露的底层错误来源。
impl Error for ParameterValidationError {}

/// 严格校验全部结构化值，并按 Definition 顺序返回确定的规范化结果。
pub fn validate_parameter_values(
    definitions: &[ParameterDefinition],
    values: &ParameterValues,
) -> Result<NormalizedParameters, ParameterValidationError> {
    validate_definitions(definitions)?;

    let declared_keys: BTreeSet<&str> = definitions.iter().map(ParameterDefinition::key).collect();
    if let Some(unknown_key) = values
        .keys()
        .find(|key| !declared_keys.contains(key.as_str()))
    {
        return Err(ParameterValidationError::new(
            unknown_key,
            ParameterErrorCode::UnknownKey,
        ));
    }

    let mut entries = Vec::with_capacity(definitions.len());
    for definition in definitions {
        let submitted_value = values.get(definition.key());
        let value = match submitted_value {
            Some(value) => Some(normalize_present_value(definition, value)?),
            None if definition.base().required => {
                return Err(ParameterValidationError::new(
                    definition.key(),
                    ParameterErrorCode::MissingRequired,
                ));
            }
            None => definition
                .default_parameter_value()
                .as_ref()
                .map(|value| normalize_present_value(definition, value))
                .transpose()?,
        };
        entries.push(NormalizedParameter {
            key: definition.key().to_owned(),
            value,
        });
    }

    Ok(NormalizedParameters { entries })
}

/// 验证 Definition key、约束关系和默认值，避免无效 Schema 产生不稳定行为。
fn validate_definitions(
    definitions: &[ParameterDefinition],
) -> Result<(), ParameterValidationError> {
    let mut keys = BTreeSet::new();
    for definition in definitions {
        let key = definition.key();
        if !is_valid_parameter_key(key) || definition.base().label.is_empty() {
            return Err(ParameterValidationError::new(
                key,
                ParameterErrorCode::InvalidDefinition,
            ));
        }
        if !keys.insert(key) {
            return Err(ParameterValidationError::new(
                key,
                ParameterErrorCode::DuplicateDefinitionKey,
            ));
        }

        validate_definition_constraints(definition)?;
        if let Some(default_value) = definition.default_parameter_value() {
            normalize_present_value(definition, &default_value).map_err(|_| {
                ParameterValidationError::new(key, ParameterErrorCode::InvalidDefinition)
            })?;
        }
    }
    Ok(())
}

/// 验证单个 Definition 内互相关联、无法由 Rust 类型单独保证的约束。
fn validate_definition_constraints(
    definition: &ParameterDefinition,
) -> Result<(), ParameterValidationError> {
    let invalid =
        || ParameterValidationError::new(definition.key(), ParameterErrorCode::InvalidDefinition);

    match definition {
        ParameterDefinition::Text(definition) => {
            if matches!(
                (definition.min_length, definition.max_length),
                (Some(min), Some(max)) if min > max
            ) {
                return Err(invalid());
            }
        }
        ParameterDefinition::Number(definition) => {
            if definition.min.is_some_and(|value| !value.is_finite())
                || definition.max.is_some_and(|value| !value.is_finite())
                || definition
                    .step
                    .is_some_and(|value| !value.is_finite() || value <= 0.0)
                || matches!((definition.min, definition.max), (Some(min), Some(max)) if min > max)
            {
                return Err(invalid());
            }
        }
        ParameterDefinition::Boolean(_) => {}
        ParameterDefinition::Select(definition) => {
            let unique_options: BTreeSet<&str> =
                definition.options.iter().map(String::as_str).collect();
            if definition.options.is_empty()
                || unique_options.len() != definition.options.len()
                || definition.options.iter().any(String::is_empty)
            {
                return Err(invalid());
            }
        }
        ParameterDefinition::Folder(_) => {}
        ParameterDefinition::Folders(definition) => {
            if matches!(
                (definition.min_items, definition.max_items),
                (Some(min), Some(max)) if min > max
            ) {
                return Err(invalid());
            }
        }
    }

    Ok(())
}

/// 按当前 Definition 校验一个已提交或默认的非缺省值。
fn normalize_present_value(
    definition: &ParameterDefinition,
    value: &ParameterValue,
) -> Result<NormalizedParameterValue, ParameterValidationError> {
    let key = definition.key();
    match (definition, value) {
        (ParameterDefinition::Text(definition), ParameterValue::Text(value)) => {
            let length = value.chars().count();
            if definition.base.required && value.is_empty() {
                return Err(ParameterValidationError::new(
                    key,
                    ParameterErrorCode::EmptyRequired,
                ));
            }
            if definition
                .min_length
                .is_some_and(|minimum| length < minimum)
            {
                return Err(ParameterValidationError::new(
                    key,
                    ParameterErrorCode::TextTooShort,
                ));
            }
            if definition
                .max_length
                .is_some_and(|maximum| length > maximum)
            {
                return Err(ParameterValidationError::new(
                    key,
                    ParameterErrorCode::TextTooLong,
                ));
            }
            Ok(NormalizedParameterValue::Text(value.clone()))
        }
        (ParameterDefinition::Number(definition), ParameterValue::Number(value)) => {
            normalize_number(definition, *value)
        }
        (ParameterDefinition::Boolean(_), ParameterValue::Boolean(value)) => {
            Ok(NormalizedParameterValue::Boolean(*value))
        }
        (ParameterDefinition::Select(definition), ParameterValue::Text(value)) => {
            if !definition.options.iter().any(|option| option == value) {
                return Err(ParameterValidationError::new(
                    key,
                    ParameterErrorCode::SelectOptionInvalid,
                ));
            }
            Ok(NormalizedParameterValue::Select(value.clone()))
        }
        (ParameterDefinition::Folder(definition), ParameterValue::Text(value)) => {
            normalize_folder_path(key, value, definition.must_exist)
                .map(NormalizedParameterValue::Folder)
        }
        (ParameterDefinition::Folders(definition), ParameterValue::Array(values)) => {
            if definition.base.required && values.is_empty() {
                return Err(ParameterValidationError::new(
                    key,
                    ParameterErrorCode::EmptyRequired,
                ));
            }
            if definition
                .min_items
                .is_some_and(|minimum| values.len() < minimum)
            {
                return Err(ParameterValidationError::new(
                    key,
                    ParameterErrorCode::FoldersTooFew,
                ));
            }
            if definition
                .max_items
                .is_some_and(|maximum| values.len() > maximum)
            {
                return Err(ParameterValidationError::new(
                    key,
                    ParameterErrorCode::FoldersTooMany,
                ));
            }

            let mut normalized_values = Vec::with_capacity(values.len());
            for item in values {
                let ParameterValue::Text(path) = item else {
                    return Err(ParameterValidationError::new(
                        key,
                        ParameterErrorCode::FoldersItemInvalidType,
                    ));
                };
                normalized_values.push(normalize_folder_path(key, path, definition.must_exist)?);
            }
            Ok(NormalizedParameterValue::Folders(normalized_values))
        }
        _ => Err(ParameterValidationError::new(
            key,
            ParameterErrorCode::InvalidType,
        )),
    }
}

/// 校验一个有限 Number 的范围和相对步长。
fn normalize_number(
    definition: &NumberParameterDefinition,
    value: f64,
) -> Result<NormalizedParameterValue, ParameterValidationError> {
    let key = &definition.base.key;
    if !value.is_finite() {
        return Err(ParameterValidationError::new(
            key,
            ParameterErrorCode::NumberNotFinite,
        ));
    }
    if definition.min.is_some_and(|minimum| value < minimum) {
        return Err(ParameterValidationError::new(
            key,
            ParameterErrorCode::NumberBelowMinimum,
        ));
    }
    if definition.max.is_some_and(|maximum| value > maximum) {
        return Err(ParameterValidationError::new(
            key,
            ParameterErrorCode::NumberAboveMaximum,
        ));
    }
    if let Some(step) = definition.step {
        let base = definition.min.unwrap_or(0.0);
        let quotient = (value - base) / step;
        let nearest_value = base + quotient.round() * step;
        let magnitude_tolerance =
            f64::EPSILON * value.abs().max(base.abs()).max(step.abs()).max(1.0) * 8.0;
        // 容差保留普通浮点运算误差，但必须显著小于半步，避免大数放大后吞掉真实半步偏移。
        let tolerance = magnitude_tolerance.min(step * 0.25);
        if !quotient.is_finite()
            || !nearest_value.is_finite()
            || (value - nearest_value).abs() > tolerance
        {
            return Err(ParameterValidationError::new(
                key,
                ParameterErrorCode::NumberStepMismatch,
            ));
        }
    }
    Ok(NormalizedParameterValue::Number(value))
}

/// 词法规范化绝对目录文本，并可选读取目标根对象元数据确认它是目录。
fn normalize_folder_path(
    key: &str,
    value: &str,
    must_exist: bool,
) -> Result<String, ParameterValidationError> {
    if value.is_empty() || value.contains('\0') {
        return Err(ParameterValidationError::new(
            key,
            ParameterErrorCode::FolderPathInvalid,
        ));
    }

    let path = Path::new(value);
    if !path.is_absolute() {
        return Err(ParameterValidationError::new(
            key,
            ParameterErrorCode::FolderPathNotAbsolute,
        ));
    }

    let normalized = normalize_absolute_path(key, path)?;
    if must_exist {
        let metadata = fs::metadata(&normalized).map_err(|error| {
            let code = if error.kind() == std::io::ErrorKind::NotFound {
                ParameterErrorCode::FolderNotFound
            } else {
                ParameterErrorCode::FolderUnavailable
            };
            ParameterValidationError::new(key, code)
        })?;
        if !metadata.is_dir() {
            return Err(ParameterValidationError::new(
                key,
                ParameterErrorCode::FolderNotDirectory,
            ));
        }
    }

    normalized
        .to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| ParameterValidationError::new(key, ParameterErrorCode::FolderPathInvalid))
}

/// 折叠 `.`、重复分隔符和可解析的 `..`，但拒绝越过绝对路径根。
fn normalize_absolute_path(key: &str, path: &Path) -> Result<PathBuf, ParameterValidationError> {
    let mut normalized = PathBuf::new();
    let mut normal_component_count = 0_usize;
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if normal_component_count == 0 {
                    return Err(ParameterValidationError::new(
                        key,
                        ParameterErrorCode::FolderPathEscapesRoot,
                    ));
                }
                normalized.pop();
                normal_component_count -= 1;
            }
            Component::Normal(part) => {
                normalized.push(part);
                normal_component_count += 1;
            }
        }
    }
    Ok(normalized)
}

/// 判断 key 是否可被受限模板无歧义引用，并保留 `this` 给 each 当前项。
pub(crate) fn is_valid_parameter_key(key: &str) -> bool {
    if key == "this" {
        return false;
    }
    let mut characters = key.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    //! 六类参数的成功、类型错误、约束错误和根目录元数据检查测试。

    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::{
        validate_parameter_values, BooleanParameterDefinition, FolderParameterDefinition,
        FoldersParameterDefinition, NormalizedParameterValue, NumberParameterDefinition,
        ParameterBase, ParameterDefinition, ParameterErrorCode, ParameterValue,
        SelectParameterDefinition, TextParameterDefinition,
    };

    /// 创建测试 Definition 使用的最小共享元数据。
    fn base(key: &str, required: bool) -> ParameterBase {
        ParameterBase {
            key: key.to_owned(),
            label: format!("{key} 参数"),
            description: None,
            required,
            remember: false,
        }
    }

    /// 创建覆盖六类参数且顺序刻意不同于字典序的测试 Definition。
    fn all_definitions(must_exist: bool) -> Vec<ParameterDefinition> {
        vec![
            ParameterDefinition::Text(TextParameterDefinition {
                base: base("text", true),
                default_value: None,
                min_length: Some(1),
                max_length: Some(64),
                placeholder: None,
            }),
            ParameterDefinition::Number(NumberParameterDefinition {
                base: base("count", true),
                default_value: None,
                min: Some(0.0),
                max: Some(10.0),
                step: Some(2.0),
            }),
            ParameterDefinition::Boolean(BooleanParameterDefinition {
                base: base("enabled", true),
                default_value: false,
            }),
            ParameterDefinition::Select(SelectParameterDefinition {
                base: base("mode", true),
                options: vec!["safe".to_owned(), "fast".to_owned()],
                default_value: None,
            }),
            ParameterDefinition::Folder(FolderParameterDefinition {
                base: base("folder", true),
                must_exist,
                default_value: None,
            }),
            ParameterDefinition::Folders(FoldersParameterDefinition {
                base: base("folders", true),
                must_exist,
                min_items: Some(1),
                max_items: Some(3),
                default_value: None,
            }),
        ]
    }

    /// 把当前平台绝对目录转换成请求中的 Unicode 文本。
    fn path_text(path: PathBuf) -> String {
        path.to_string_lossy().into_owned()
    }

    /// 验证六类正常值按 Definition 顺序返回，并保留中文、空格、单引号和多路径顺序。
    #[test]
    fn validates_six_types_in_definition_order() {
        let root = std::env::temp_dir();
        let raw_folder = root.join("中文 空格").join("..").join("folder's");
        let expected_folder = root.join("folder's");
        let raw_first = root.join("列表 一").join(".");
        let expected_first = root.join("列表 一");
        let second = root.join("列表'two");
        let mut values = BTreeMap::new();
        values.insert("mode".to_owned(), ParameterValue::Text("safe".to_owned()));
        values.insert("enabled".to_owned(), ParameterValue::Boolean(true));
        values.insert(
            "folders".to_owned(),
            ParameterValue::Array(vec![
                ParameterValue::Text(path_text(raw_first)),
                ParameterValue::Text(path_text(second.clone())),
            ]),
        );
        values.insert("count".to_owned(), ParameterValue::Number(4.0));
        values.insert(
            "text".to_owned(),
            ParameterValue::Text("中文 空格 ' 单引号".to_owned()),
        );
        values.insert(
            "folder".to_owned(),
            ParameterValue::Text(path_text(raw_folder)),
        );

        let normalized =
            validate_parameter_values(&all_definitions(false), &values).expect("六类值应通过");

        assert_eq!(
            normalized
                .entries
                .iter()
                .map(|entry| entry.key.as_str())
                .collect::<Vec<_>>(),
            vec!["text", "count", "enabled", "mode", "folder", "folders"]
        );
        assert_eq!(
            normalized.get("text"),
            Some(&NormalizedParameterValue::Text(
                "中文 空格 ' 单引号".to_owned()
            ))
        );
        assert_eq!(
            normalized.get("folder"),
            Some(&NormalizedParameterValue::Folder(path_text(
                expected_folder
            )))
        );
        assert_eq!(
            normalized.get("folders"),
            Some(&NormalizedParameterValue::Folders(vec![
                path_text(expected_first),
                path_text(second),
            ]))
        );
    }

    /// 验证未知 key 与缺失 required key 都以稳定错误拒绝。
    #[test]
    fn rejects_unknown_and_missing_required_keys() {
        let definition = ParameterDefinition::Text(TextParameterDefinition {
            base: base("text", true),
            default_value: None,
            min_length: None,
            max_length: None,
            placeholder: None,
        });
        let mut unknown_values = BTreeMap::new();
        unknown_values.insert(
            "unexpected".to_owned(),
            ParameterValue::Text("value".to_owned()),
        );

        let unknown_error =
            validate_parameter_values(std::slice::from_ref(&definition), &unknown_values)
                .expect_err("未知 key 应拒绝");
        assert_eq!(unknown_error.key, "unexpected");
        assert_eq!(unknown_error.code, ParameterErrorCode::UnknownKey);

        let missing_error = validate_parameter_values(&[definition], &BTreeMap::new())
            .expect_err("缺失 required key 应拒绝");
        assert_eq!(missing_error.key, "text");
        assert_eq!(missing_error.code, ParameterErrorCode::MissingRequired);
    }

    /// 验证 required Text 显式提交空文本时不会被当作有效值。
    #[test]
    fn rejects_empty_required_text() {
        let definition = ParameterDefinition::Text(TextParameterDefinition {
            base: base("text", true),
            default_value: None,
            min_length: None,
            max_length: None,
            placeholder: None,
        });
        let values = BTreeMap::from([("text".to_owned(), ParameterValue::Text(String::new()))]);

        let error =
            validate_parameter_values(&[definition], &values).expect_err("required 空文本应拒绝");
        assert_eq!(error.code, ParameterErrorCode::EmptyRequired);
    }

    /// 验证六类 Definition 均拒绝不匹配的 JSON 值类别，包括 Object 和数组项类型。
    #[test]
    fn rejects_wrong_structured_value_types() {
        let definitions = all_definitions(false);
        let wrong_values = vec![
            ParameterValue::Object(BTreeMap::new()),
            ParameterValue::Text("4".to_owned()),
            ParameterValue::Text("true".to_owned()),
            ParameterValue::Boolean(true),
            ParameterValue::Array(Vec::new()),
            ParameterValue::Text("not-an-array".to_owned()),
        ];

        for (definition, wrong_value) in definitions.iter().zip(wrong_values) {
            let values = BTreeMap::from([(definition.key().to_owned(), wrong_value)]);
            let error = validate_parameter_values(std::slice::from_ref(definition), &values)
                .expect_err("错误 JSON 类型应拒绝");
            assert_eq!(error.code, ParameterErrorCode::InvalidType);
        }

        let folders = definitions.last().expect("应有 Folders Definition").clone();
        let invalid_items = BTreeMap::from([(
            "folders".to_owned(),
            ParameterValue::Array(vec![ParameterValue::Number(1.0)]),
        )]);
        let error = validate_parameter_values(&[folders], &invalid_items)
            .expect_err("Folders 非 String 项应拒绝");
        assert_eq!(error.code, ParameterErrorCode::FoldersItemInvalidType);
    }

    /// 验证 Number 拒绝非有限值、上下界之外的值和不满足 step 的值。
    #[test]
    fn rejects_non_finite_range_and_step_numbers() {
        let definition = ParameterDefinition::Number(NumberParameterDefinition {
            base: base("count", true),
            default_value: None,
            min: Some(0.0),
            max: Some(10.0),
            step: Some(2.0),
        });
        let cases = [
            (f64::NAN, ParameterErrorCode::NumberNotFinite),
            (-1.0, ParameterErrorCode::NumberBelowMinimum),
            (11.0, ParameterErrorCode::NumberAboveMaximum),
            (1.5, ParameterErrorCode::NumberStepMismatch),
        ];

        for (value, expected_code) in cases {
            let values = BTreeMap::from([("count".to_owned(), ParameterValue::Number(value))]);
            let error = validate_parameter_values(std::slice::from_ref(&definition), &values)
                .expect_err("非法 Number 应拒绝");
            assert_eq!(error.code, expected_code);
        }

        let large_definition = ParameterDefinition::Number(NumberParameterDefinition {
            base: base("count", true),
            default_value: None,
            min: Some(0.0),
            max: None,
            step: Some(1.0),
        });
        let large_off_step = BTreeMap::from([(
            "count".to_owned(),
            ParameterValue::Number(1_000_000_000_000_000.5),
        )]);
        let error = validate_parameter_values(&[large_definition], &large_off_step)
            .expect_err("大数值的半步偏移仍应拒绝");
        assert_eq!(error.code, ParameterErrorCode::NumberStepMismatch);

        let decimal_definition = ParameterDefinition::Number(NumberParameterDefinition {
            base: base("count", true),
            default_value: None,
            min: Some(0.0),
            max: Some(1.0),
            step: Some(0.1),
        });
        let decimal_rounding =
            BTreeMap::from([("count".to_owned(), ParameterValue::Number(0.1 + 0.2))]);
        validate_parameter_values(&[decimal_definition], &decimal_rounding)
            .expect("普通十进制浮点舍入误差仍应通过 step 校验");
    }

    /// 验证 Select 只能接受固定 options 中的精确值。
    #[test]
    fn rejects_select_value_outside_options() {
        let definition = ParameterDefinition::Select(SelectParameterDefinition {
            base: base("mode", true),
            options: vec!["safe".to_owned(), "fast".to_owned()],
            default_value: None,
        });
        let values = BTreeMap::from([("mode".to_owned(), ParameterValue::Text("SAFE".to_owned()))]);

        let error =
            validate_parameter_values(&[definition], &values).expect_err("非法 Select 应拒绝");
        assert_eq!(error.code, ParameterErrorCode::SelectOptionInvalid);
    }

    /// 验证 Folders 同时执行 min_items、max_items 和 required 空集合约束。
    #[test]
    fn rejects_invalid_folders_item_counts() {
        let definition = ParameterDefinition::Folders(FoldersParameterDefinition {
            base: base("folders", true),
            must_exist: false,
            min_items: Some(2),
            max_items: Some(3),
            default_value: None,
        });
        let root = std::env::temp_dir();
        let one = ParameterValue::Array(vec![ParameterValue::Text(path_text(root.join("one")))]);
        let four = ParameterValue::Array(
            ["one", "two", "three", "four"]
                .into_iter()
                .map(|name| ParameterValue::Text(path_text(root.join(name))))
                .collect(),
        );

        let too_few = validate_parameter_values(
            std::slice::from_ref(&definition),
            &BTreeMap::from([("folders".to_owned(), one)]),
        )
        .expect_err("少于 min_items 应拒绝");
        assert_eq!(too_few.code, ParameterErrorCode::FoldersTooFew);

        let too_many = validate_parameter_values(
            std::slice::from_ref(&definition),
            &BTreeMap::from([("folders".to_owned(), four)]),
        )
        .expect_err("多于 max_items 应拒绝");
        assert_eq!(too_many.code, ParameterErrorCode::FoldersTooMany);

        let empty = validate_parameter_values(
            &[ParameterDefinition::Folders(FoldersParameterDefinition {
                base: base("folders", true),
                must_exist: false,
                min_items: None,
                max_items: None,
                default_value: None,
            })],
            &BTreeMap::from([("folders".to_owned(), ParameterValue::Array(Vec::new()))]),
        )
        .expect_err("required 空 Folders 应拒绝");
        assert_eq!(empty.code, ParameterErrorCode::EmptyRequired);
    }

    /// 验证 `must_exist` 只接受存在的目录根，并区分文件根与不存在根。
    #[test]
    fn checks_only_must_exist_folder_root_metadata() {
        let definition = ParameterDefinition::Folder(FolderParameterDefinition {
            base: base("folder", true),
            must_exist: true,
            default_value: None,
        });
        let existing_directory = BTreeMap::from([(
            "folder".to_owned(),
            ParameterValue::Text(path_text(std::env::temp_dir())),
        )]);
        validate_parameter_values(std::slice::from_ref(&definition), &existing_directory)
            .expect("存在的目录根应通过");

        let executable = std::env::current_exe().expect("测试进程路径应可读取");
        let file_root = BTreeMap::from([(
            "folder".to_owned(),
            ParameterValue::Text(path_text(executable.clone())),
        )]);
        let file_error = validate_parameter_values(std::slice::from_ref(&definition), &file_root)
            .expect_err("文件根不应作为 Folder");
        assert_eq!(file_error.code, ParameterErrorCode::FolderNotDirectory);

        let missing = executable
            .parent()
            .expect("测试进程应有父目录")
            .join(format!("cmdbox-missing-folder-{}", std::process::id()));
        let missing_root = BTreeMap::from([(
            "folder".to_owned(),
            ParameterValue::Text(path_text(missing)),
        )]);
        let missing_error = validate_parameter_values(&[definition], &missing_root)
            .expect_err("不存在的目录根应拒绝");
        assert_eq!(missing_error.code, ParameterErrorCode::FolderNotFound);
    }

    /// 验证 Folder 拒绝相对路径以及试图越过绝对根的父级组件。
    #[test]
    fn rejects_relative_and_root_escaping_folder_paths() {
        let definition = ParameterDefinition::Folder(FolderParameterDefinition {
            base: base("folder", true),
            must_exist: false,
            default_value: None,
        });
        let relative = BTreeMap::from([(
            "folder".to_owned(),
            ParameterValue::Text("relative-folder".to_owned()),
        )]);
        let relative_error =
            validate_parameter_values(std::slice::from_ref(&definition), &relative)
                .expect_err("相对路径应拒绝");
        assert_eq!(
            relative_error.code,
            ParameterErrorCode::FolderPathNotAbsolute
        );

        let root = std::env::temp_dir()
            .ancestors()
            .last()
            .expect("绝对临时目录应有根")
            .to_path_buf();
        let escaping = BTreeMap::from([(
            "folder".to_owned(),
            ParameterValue::Text(path_text(root.join("..").join("escape"))),
        )]);
        let escaping_error =
            validate_parameter_values(&[definition], &escaping).expect_err("越过绝对根应拒绝");
        assert_eq!(
            escaping_error.code,
            ParameterErrorCode::FolderPathEscapesRoot
        );
    }
}
