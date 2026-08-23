//! Canonical Execution Spec 的确定性二进制编码与 SHA-256。
//!
//! 本模块把一次已验证、已渲染但尚未产生任何外部副作用的执行事实编码为带 Schema Version
//! 的 length-prefixed 二进制记录。编码不依赖 JSON、HashMap 迭代或本地化显示文本；Preview
//! 与 Run 复验都通过同一构造路径计算 Hash。

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

use super::parameter::{NormalizedParameterValue, NormalizedParameters};

/// Canonical Execution Spec 的稳定格式身份，避免其他二进制载荷与本格式混淆。
const EXECUTION_SPEC_FORMAT: &[u8] = b"cmdbox.execution-spec";

/// 一次 Preview/Run 都必须绑定的完整规范化执行事实。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CanonicalExecutionSpec {
    /// Canonical 编码格式版本；字段或编码规则变化时必须递增。
    pub(crate) schema_version: u32,
    /// Command Block 的稳定 ID。
    pub(crate) command_block_id: String,
    /// 当前 Definition revision。
    pub(crate) revision: u64,
    /// 声明 Runner 的稳定协议标识。
    pub(crate) runner_type: String,
    /// Rust 从系统能力解析出的确定 Runner 绝对路径。
    pub(crate) runner_executable: PathBuf,
    /// 动态脚本路径之前的固定 Runner 选项，顺序属于执行语义。
    pub(crate) runner_fixed_options: Vec<OsString>,
    /// CMD `/C` 后的固定 raw command tail；PowerShell 为 `None`。
    pub(crate) runner_raw_command_tail: Option<OsString>,
    /// 对包含编码前导的完整最终 Artifact 字节计算的 SHA-256。
    pub(crate) artifact_hash: [u8; 32],
    /// 严格按 Parameter Definition 顺序保存的规范化参数。
    pub(crate) normalized_parameters: NormalizedParameters,
    /// Runner 启动时使用的确定工作目录。
    pub(crate) working_directory: PathBuf,
    /// Command Block 显式声明的非敏感环境变量，按 key 排序。
    pub(crate) explicit_environment: BTreeMap<String, String>,
    /// Runner 固定环境与非空参数私有绑定，值按原始 UTF-16 编码。
    pub(crate) internal_environment: BTreeMap<String, OsString>,
    /// 当前 Safety Policy 语义版本。
    pub(crate) safety_policy_version: u32,
    /// 当前 Outcome Policy 语义版本。
    pub(crate) outcome_policy_version: u32,
}

impl CanonicalExecutionSpec {
    /// 对完整 Canonical 二进制记录计算 SHA-256。
    pub(crate) fn hash(&self) -> [u8; 32] {
        Sha256::digest(self.encode()).into()
    }

    /// 返回供 IPC 传递和稳定比较的小写十六进制 SHA-256。
    pub(crate) fn hash_hex(&self) -> String {
        encode_hash_hex(&self.hash())
    }

    /// 按固定字段顺序生成带格式身份与 Schema Version 的 length-prefixed 二进制记录。
    fn encode(&self) -> Vec<u8> {
        let mut record = CanonicalWriter::new();
        record.field("format", EXECUTION_SPEC_FORMAT);
        record.field("schemaVersion", &self.schema_version.to_le_bytes());
        record.field("commandBlockId", self.command_block_id.as_bytes());
        record.field("revision", &self.revision.to_le_bytes());
        record.field("runnerType", self.runner_type.as_bytes());
        record.field(
            "runnerExecutable",
            &encode_windows_os_string(self.runner_executable.as_os_str()),
        );
        record.field(
            "runnerFixedOptions",
            &encode_os_string_sequence(&self.runner_fixed_options),
        );
        record.field(
            "runnerRawCommandTail",
            &encode_optional_os_string(self.runner_raw_command_tail.as_ref()),
        );
        record.field("artifactHash", &self.artifact_hash);
        record.field(
            "normalizedParameters",
            &encode_normalized_parameters(&self.normalized_parameters),
        );
        record.field(
            "workingDirectory",
            &encode_windows_os_string(self.working_directory.as_os_str()),
        );
        record.field(
            "explicitEnvironment",
            &encode_environment(&self.explicit_environment),
        );
        record.field(
            "internalEnvironment",
            &encode_os_environment(&self.internal_environment),
        );
        record.field(
            "safetyPolicyVersion",
            &self.safety_policy_version.to_le_bytes(),
        );
        record.field(
            "outcomePolicyVersion",
            &self.outcome_policy_version.to_le_bytes(),
        );
        record.finish()
    }
}

/// 只提供 length-prefixed 字段写入的最小 Canonical 编码器。
struct CanonicalWriter {
    /// 已按调用顺序完成编码的二进制字节。
    bytes: Vec<u8>,
}

impl CanonicalWriter {
    /// 创建空的 Canonical 记录。
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    /// 写入字段名和字段载荷，两者都带八字节小端长度前缀。
    fn field(&mut self, name: &str, payload: &[u8]) {
        push_length_prefixed(&mut self.bytes, name.as_bytes());
        push_length_prefixed(&mut self.bytes, payload);
    }

    /// 完成当前记录并交出唯一字节所有权。
    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

/// 把任意载荷写为 `u64 little-endian length + bytes`，消除字段拼接歧义。
fn push_length_prefixed(output: &mut Vec<u8>, payload: &[u8]) {
    let length = payload.len() as u64;
    output.extend_from_slice(&length.to_le_bytes());
    output.extend_from_slice(payload);
}

/// 将 Windows `OsStr` 按原始 UTF-16 code unit 和小端顺序编码，避免有损路径转换。
fn encode_windows_os_string(value: &OsStr) -> Vec<u8> {
    let units = value.encode_wide().collect::<Vec<_>>();
    let mut bytes = Vec::with_capacity(units.len().saturating_mul(2));
    for unit in units {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

/// 按固定顺序编码 Runner options，每个 option 都拥有独立长度边界。
fn encode_os_string_sequence(values: &[OsString]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(values.len() as u64).to_le_bytes());
    for value in values {
        push_length_prefixed(&mut bytes, &encode_windows_os_string(value));
    }
    bytes
}

/// 编码可选 Windows 字符串并保留 `None` 与空字符串的区别。
fn encode_optional_os_string(value: Option<&OsString>) -> Vec<u8> {
    let mut bytes = Vec::new();
    match value {
        Some(value) => {
            bytes.push(1);
            push_length_prefixed(&mut bytes, &encode_windows_os_string(value));
        }
        None => bytes.push(0),
    }
    bytes
}

/// 按 Definition 顺序编码参数 key、存在状态、类型和值。
fn encode_normalized_parameters(parameters: &NormalizedParameters) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(parameters.entries.len() as u64).to_le_bytes());
    for entry in &parameters.entries {
        let mut encoded_entry = CanonicalWriter::new();
        encoded_entry.field("key", entry.key.as_bytes());
        match &entry.value {
            Some(value) => {
                encoded_entry.field("present", &[1]);
                encode_parameter_value(&mut encoded_entry, value);
            }
            None => {
                encoded_entry.field("present", &[0]);
                encoded_entry.field("type", b"none");
                encoded_entry.field("value", &[]);
            }
        }
        push_length_prefixed(&mut bytes, &encoded_entry.finish());
    }
    bytes
}

/// 编码单个规范化参数的稳定类型标识和值。
fn encode_parameter_value(writer: &mut CanonicalWriter, value: &NormalizedParameterValue) {
    match value {
        NormalizedParameterValue::Text(value) => {
            writer.field("type", b"text");
            writer.field("value", value.as_bytes());
        }
        NormalizedParameterValue::Number(value) => {
            writer.field("type", b"number");
            let canonical_value = if *value == 0.0 { 0.0 } else { *value };
            writer.field("value", &canonical_value.to_bits().to_le_bytes());
        }
        NormalizedParameterValue::Boolean(value) => {
            writer.field("type", b"boolean");
            writer.field("value", &[u8::from(*value)]);
        }
        NormalizedParameterValue::Select(value) => {
            writer.field("type", b"select");
            writer.field("value", value.as_bytes());
        }
        NormalizedParameterValue::Folder(value) => {
            writer.field("type", b"folder");
            writer.field("value", value.as_bytes());
        }
        NormalizedParameterValue::Folders(values) => {
            writer.field("type", b"folders");
            writer.field("value", &encode_string_sequence(values));
        }
    }
}

/// 按原有业务顺序编码字符串序列，并为每项保留长度边界。
fn encode_string_sequence(values: &[String]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(values.len() as u64).to_le_bytes());
    for value in values {
        push_length_prefixed(&mut bytes, value.as_bytes());
    }
    bytes
}

/// 按 `BTreeMap` key 顺序编码显式环境，使插入顺序不改变 Execution Spec Hash。
fn encode_environment(environment: &BTreeMap<String, String>) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(environment.len() as u64).to_le_bytes());
    for (key, value) in environment {
        let mut encoded_entry = CanonicalWriter::new();
        encoded_entry.field("key", key.as_bytes());
        encoded_entry.field("value", value.as_bytes());
        push_length_prefixed(&mut bytes, &encoded_entry.finish());
    }
    bytes
}

/// 按稳定 key 顺序编码内部环境，保留 Windows 值的原始 UTF-16 单元。
fn encode_os_environment(environment: &BTreeMap<String, OsString>) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(environment.len() as u64).to_le_bytes());
    for (key, value) in environment {
        let mut encoded_entry = CanonicalWriter::new();
        encoded_entry.field("key", key.as_bytes());
        encoded_entry.field("value", &encode_windows_os_string(value));
        push_length_prefixed(&mut bytes, &encoded_entry.finish());
    }
    bytes
}

/// 将 32 字节 SHA-256 编码为固定 64 字符小写十六进制文本。
fn encode_hash_hex(hash: &[u8; 32]) -> String {
    const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(64);
    for byte in hash {
        encoded.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        encoded.push(HEX_DIGITS[(byte & 0x0F) as usize] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    //! Canonical 字段覆盖、Map 顺序与长度边界测试。

    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::CanonicalExecutionSpec;
    use crate::execution::parameter::{
        NormalizedParameter, NormalizedParameterValue, NormalizedParameters,
    };

    /// 创建所有当前 Hash 组件都有可辨识值的 Canonical Spec。
    fn fixture() -> CanonicalExecutionSpec {
        CanonicalExecutionSpec {
            schema_version: 2,
            command_block_id: "builtin.test".to_owned(),
            revision: 3,
            runner_type: "windowsPowerShell".to_owned(),
            runner_executable: PathBuf::from(
                r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
            ),
            runner_fixed_options: vec![OsString::from("-NoLogo"), OsString::from("-File")],
            runner_raw_command_tail: None,
            artifact_hash: [7; 32],
            normalized_parameters: NormalizedParameters {
                entries: vec![NormalizedParameter {
                    key: "text".to_owned(),
                    value: Some(NormalizedParameterValue::Text("中文".to_owned())),
                }],
            },
            working_directory: PathBuf::from(r"C:\Temp\CmdBox"),
            explicit_environment: BTreeMap::from([
                ("CMDBOX_FIRST".to_owned(), "一".to_owned()),
                ("CMDBOX_SECOND".to_owned(), "two".to_owned()),
            ]),
            internal_environment: BTreeMap::from([(
                "CMDBOX_INTERNAL_CHCP".to_owned(),
                OsString::from(r"C:\Windows\System32\chcp.com"),
            )]),
            safety_policy_version: 4,
            outcome_policy_version: 5,
        }
    }

    /// 验证每个要求覆盖的组件单独变化时，旧 Hash 都会失效。
    #[test]
    fn every_execution_spec_component_changes_hash() {
        let baseline = fixture();
        let baseline_hash = baseline.hash();
        let mut variants = Vec::new();

        let mut changed = baseline.clone();
        changed.schema_version += 1;
        variants.push(changed);
        let mut changed = baseline.clone();
        changed.command_block_id.push_str(".changed");
        variants.push(changed);
        let mut changed = baseline.clone();
        changed.revision += 1;
        variants.push(changed);
        let mut changed = baseline.clone();
        changed.runner_type = "cmd".to_owned();
        variants.push(changed);
        let mut changed = baseline.clone();
        changed.runner_executable = PathBuf::from(r"C:\Windows\System32\cmd.exe");
        variants.push(changed);
        let mut changed = baseline.clone();
        changed
            .runner_fixed_options
            .push(OsString::from("-Changed"));
        variants.push(changed);
        let mut changed = baseline.clone();
        changed.runner_raw_command_tail = Some(OsString::from(r#"""!ARTIFACT!"""#));
        variants.push(changed);
        let mut changed = baseline.clone();
        changed.artifact_hash[0] ^= 1;
        variants.push(changed);
        let mut changed = baseline.clone();
        changed.normalized_parameters.entries[0].value =
            Some(NormalizedParameterValue::Text("另一值".to_owned()));
        variants.push(changed);
        let mut changed = baseline.clone();
        changed.working_directory = PathBuf::from(r"D:\Work");
        variants.push(changed);
        let mut changed = baseline.clone();
        changed
            .explicit_environment
            .insert("CMDBOX_THIRD".to_owned(), "3".to_owned());
        variants.push(changed);
        let mut changed = baseline.clone();
        changed.internal_environment.insert(
            "CMDBOX_INTERNAL_VALUE_00000000".to_owned(),
            OsString::from("literal"),
        );
        variants.push(changed);
        let mut changed = baseline.clone();
        changed.safety_policy_version += 1;
        variants.push(changed);
        let mut changed = baseline.clone();
        changed.outcome_policy_version += 1;
        variants.push(changed);

        for variant in variants {
            assert_ne!(variant.hash(), baseline_hash);
        }
    }

    /// 验证相同环境键值的不同插入顺序不会改变 Canonical Hash。
    #[test]
    fn environment_map_insertion_order_does_not_change_hash() {
        let first = fixture();
        let mut second = fixture();
        second.explicit_environment = BTreeMap::new();
        second
            .explicit_environment
            .insert("CMDBOX_SECOND".to_owned(), "two".to_owned());
        second
            .explicit_environment
            .insert("CMDBOX_FIRST".to_owned(), "一".to_owned());

        assert_eq!(first.hash(), second.hash());
        assert_eq!(first.hash_hex().len(), 64);

        let mut third = fixture();
        third.internal_environment = BTreeMap::new();
        third.internal_environment.insert(
            "CMDBOX_INTERNAL_VALUE_00000000".to_owned(),
            OsString::from("literal"),
        );
        third.internal_environment.insert(
            "CMDBOX_INTERNAL_CHCP".to_owned(),
            OsString::from(r"C:\Windows\System32\chcp.com"),
        );
        let mut fourth = third.clone();
        fourth.internal_environment = BTreeMap::new();
        fourth.internal_environment.insert(
            "CMDBOX_INTERNAL_CHCP".to_owned(),
            OsString::from(r"C:\Windows\System32\chcp.com"),
        );
        fourth.internal_environment.insert(
            "CMDBOX_INTERNAL_VALUE_00000000".to_owned(),
            OsString::from("literal"),
        );
        assert_eq!(third.hash(), fourth.hash());
    }

    /// 验证 length prefix 能区分内容拼接后相同但字段边界不同的参数序列。
    #[test]
    fn length_prefix_preserves_parameter_boundaries() {
        let mut first = fixture();
        first.normalized_parameters.entries = vec![
            NormalizedParameter {
                key: "a".to_owned(),
                value: Some(NormalizedParameterValue::Text("bc".to_owned())),
            },
            NormalizedParameter {
                key: "d".to_owned(),
                value: Some(NormalizedParameterValue::Text(String::new())),
            },
        ];
        let mut second = fixture();
        second.normalized_parameters.entries = vec![
            NormalizedParameter {
                key: "ab".to_owned(),
                value: Some(NormalizedParameterValue::Text("c".to_owned())),
            },
            NormalizedParameter {
                key: "d".to_owned(),
                value: Some(NormalizedParameterValue::Text(String::new())),
            },
        ];

        assert_ne!(first.hash(), second.hash());
    }
}
