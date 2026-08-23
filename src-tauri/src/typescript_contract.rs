//! Rust 公开 wire DTO 的 TypeScript 生成与漂移检查。
//!
//! 本模块只在 Rust 测试构建中存在。普通测试只把契约生成到当前测试拥有的临时目录并与
//! `src/generated` 比较；唯一 ignored 测试才显式更新仓库内生成文件，避免日常测试静默改源码。

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use ts_rs::{Config, TS};

use crate::execution::planner::{
    CommandBlockDetails, CommandBlockSummary, PreviewCommandRequest, PreviewCommandResponse,
    VerifyRunRequest,
};
use crate::ipc::execution::{
    ApiError, CancelExecutionResponse, ExecutionStreamEvent, RunCommandResponse,
};

/// 生成文件顶部的职责与编辑约束说明。
const GENERATED_NOTICE: &str = "// 此文件由 Rust serde DTO 生成；请勿手工编辑。\n\n";

/// 一个本测试进程唯一拥有、退出时精确清理的契约临时目录。
struct TemporaryContractDirectory {
    /// 只位于系统临时目录 `CmdBox` 子树下的唯一绝对路径。
    path: PathBuf,
}

impl TemporaryContractDirectory {
    /// 创建不复用其他测试内容的唯一目录。
    fn create() -> Result<Self, Box<dyn Error>> {
        let path = std::env::temp_dir()
            .join("CmdBox")
            .join(format!("typescript-contract-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    /// 返回 ts-rs 可写入的目录路径。
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryContractDirectory {
    /// 仅删除本实例创建的 UUID 子目录；失败留给操作系统临时目录策略处理。
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// 返回仓库中唯一受支持的 TypeScript Contract 目录。
fn checked_in_contract_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("src")
        .join("generated")
}

/// 使用公开 wire DTO 根白名单递归导出它们真实依赖的 TypeScript 类型。
fn generate_contract_into(output_directory: &Path) -> Result<(), Box<dyn Error>> {
    let config = Config::new()
        .with_out_dir(output_directory)
        .with_large_int("number");

    CommandBlockSummary::export_all(&config)?;
    CommandBlockDetails::export_all(&config)?;
    PreviewCommandRequest::export_all(&config)?;
    PreviewCommandResponse::export_all(&config)?;
    VerifyRunRequest::export_all(&config)?;
    RunCommandResponse::export_all(&config)?;
    CancelExecutionResponse::export_all(&config)?;
    ExecutionStreamEvent::export_all(&config)?;
    ApiError::export_all(&config)?;

    finalize_generated_files(output_directory)?;
    Ok(())
}

/// 统一 ts-rs 产物换行与行尾空白，并补充生成标记，不改变任何类型声明。
fn finalize_generated_files(output_directory: &Path) -> Result<(), Box<dyn Error>> {
    for path in typescript_files(output_directory)?.values() {
        let contents = fs::read_to_string(path)?;
        let normalized_body = contents
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(
            path,
            format!("{GENERATED_NOTICE}{}\n", normalized_body.trim_end()),
        )?;
    }
    Ok(())
}

/// 读取一个目录内按相对文件名排序的全部顶层 TypeScript 文件。
fn typescript_files(directory: &Path) -> Result<BTreeMap<String, PathBuf>, Box<dyn Error>> {
    let mut files = BTreeMap::new();
    if !directory.is_dir() {
        return Ok(files);
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("ts") {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        files.insert(name, path);
    }
    Ok(files)
}

/// 把 Windows 与 Unix 文本换行收敛成同一比较语义。
fn normalized_newlines(bytes: Vec<u8>) -> Result<String, Box<dyn Error>> {
    Ok(String::from_utf8(bytes)?.replace("\r\n", "\n"))
}

/// 比较生成目录的精确文件集合与规范化文本内容，发现漂移时返回可操作说明。
fn assert_contract_matches(
    actual_directory: &Path,
    expected_directory: &Path,
) -> Result<(), Box<dyn Error>> {
    let actual = typescript_files(actual_directory)?;
    let expected = typescript_files(expected_directory)?;
    let actual_names = actual.keys().cloned().collect::<Vec<_>>();
    let expected_names = expected.keys().cloned().collect::<Vec<_>>();
    if actual_names != expected_names {
        return Err(format!(
            "TypeScript Contract 文件集合已漂移；生成={actual_names:?}，仓库={expected_names:?}"
        )
        .into());
    }
    for name in actual_names {
        let actual_contents =
            normalized_newlines(fs::read(actual.get(&name).expect("生成文件名应存在"))?)?;
        let expected_contents =
            normalized_newlines(fs::read(expected.get(&name).expect("仓库文件名应存在"))?)?;
        if actual_contents != expected_contents {
            return Err(format!("TypeScript Contract 内容已漂移：{name}").into());
        }
    }
    Ok(())
}

/// 用新生成的精确文件集合替换仓库中的旧生成文件，不触碰目录外内容。
fn replace_checked_in_contract(
    generated_directory: &Path,
    checked_in_directory: &Path,
) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(checked_in_directory)?;
    let generated = typescript_files(generated_directory)?;
    let checked_in = typescript_files(checked_in_directory)?;
    for (name, path) in checked_in {
        if !generated.contains_key(&name) {
            fs::remove_file(path)?;
        }
    }
    for (name, source) in generated {
        fs::copy(source, checked_in_directory.join(name))?;
    }
    Ok(())
}

/// 日常测试必须只读仓库源码，并在生成类型发生漂移时失败。
#[test]
fn generated_typescript_contract_is_current() -> Result<(), Box<dyn Error>> {
    let temporary = TemporaryContractDirectory::create()?;
    generate_contract_into(temporary.path())?;
    assert_contract_matches(temporary.path(), &checked_in_contract_directory())
}

/// 仅由显式生成命令调用，把 Rust DTO 的当前真值写入 `src/generated`。
#[test]
#[ignore = "只通过 pnpm contract:generate 显式更新仓库生成文件"]
fn regenerates_checked_in_typescript_contract() -> Result<(), Box<dyn Error>> {
    let temporary = TemporaryContractDirectory::create()?;
    generate_contract_into(temporary.path())?;
    replace_checked_in_contract(temporary.path(), &checked_in_contract_directory())
}
