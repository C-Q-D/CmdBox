//! 最终脚本字节、临时脚本租约、完整性复验与精确清理。
//!
//! `RenderedScript` 在 Preview 之前冻结最终编码字节及 SHA-256；`MaterializedScript` 再为
//! Execution 创建 CmdBox 临时根下的唯一目录和受控固定文件名。落盘后立即 flush 并按最终
//! 字节 Hash 复验，进程启动前还会再次复验；目录所有权由 RAII 管理，调用方不能替换路径。

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Windows PowerShell 5.1 识别 UTF-8 脚本所需的 BOM。
const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// CmdBox 在系统临时目录下使用的专属子目录名。
const CMDBOX_TEMP_DIRECTORY_NAME: &str = "CmdBox";

/// 临时 Windows PowerShell 脚本使用的固定文件名。
const POWERSHELL_SCRIPT_FILE_NAME: &str = "script.ps1";

/// 已冻结最终编码、受控文件类型与完整字节 Hash 的脚本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedScript {
    /// 会实际写入临时脚本的完整字节，包含 Runner 要求的编码前导。
    bytes: Vec<u8>,
    /// 由 Rust Core 固定构造的脚本文件名，不接受用户路径或文件名。
    file_name: &'static str,
    /// 对完整最终字节计算的 SHA-256，供 Preview 与落盘复验共享。
    artifact_hash: [u8; 32],
}

impl RenderedScript {
    /// 把脚本文本编码成 Windows PowerShell 5.1 所需的 UTF-8 BOM 最终字节。
    pub fn windows_powershell(script: &str) -> Self {
        let mut bytes = Vec::with_capacity(UTF8_BOM.len() + script.len());
        bytes.extend_from_slice(&UTF8_BOM);
        bytes.extend_from_slice(script.as_bytes());
        let artifact_hash = Sha256::digest(&bytes).into();
        Self {
            bytes,
            file_name: POWERSHELL_SCRIPT_FILE_NAME,
            artifact_hash,
        }
    }

    /// 返回会实际落盘和执行的完整最终字节。
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// 返回对完整最终字节计算的 SHA-256。
    pub fn artifact_hash(&self) -> [u8; 32] {
        self.artifact_hash
    }

    /// 返回由脚本类型固定的安全文件名。
    fn file_name(&self) -> &'static str {
        self.file_name
    }
}

/// Artifact 文件操作的稳定阶段标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactOperation {
    /// 创建 CmdBox 专属临时根目录。
    CreateRootDirectory,
    /// 创建本次 Execution 的唯一目录。
    CreateExecutionDirectory,
    /// 创建固定名称的脚本文件。
    CreateScriptFile,
    /// 写入或 flush 脚本文件。
    WriteScript,
    /// 读取脚本以计算或复验 Hash。
    ReadScript,
    /// 删除本次 Execution 的专属目录。
    RemoveExecutionDirectory,
}

/// 输出 Artifact 操作的稳定开发者标识。
impl Display for ArtifactOperation {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::CreateRootDirectory => "createRootDirectory",
            Self::CreateExecutionDirectory => "createExecutionDirectory",
            Self::CreateScriptFile => "createScriptFile",
            Self::WriteScript => "writeScript",
            Self::ReadScript => "readScript",
            Self::RemoveExecutionDirectory => "removeExecutionDirectory",
        };
        formatter.write_str(value)
    }
}

/// 受管脚本 Artifact 失败。
#[derive(Debug)]
pub enum ArtifactError {
    /// 文件系统操作失败，并保留具体操作、路径和底层 I/O 错误。
    Io {
        /// 失败发生的 Artifact 操作。
        operation: ArtifactOperation,
        /// 操作涉及的绝对或系统临时路径。
        path: PathBuf,
        /// 文件系统返回的原始错误。
        source: io::Error,
    },
    /// 落盘后或启动前脚本字节与冻结时的 expected SHA-256 不一致。
    IntegrityMismatch {
        /// 完整性复验失败的受管脚本路径。
        path: PathBuf,
    },
}

/// 输出面向开发者的 Artifact 错误说明。
impl Display for ArtifactError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "Script Artifact 操作 {operation} 失败（{}）：{source}",
                path.display()
            ),
            Self::IntegrityMismatch { path } => write!(
                formatter,
                "Script Artifact 完整性校验失败：{}",
                path.display()
            ),
        }
    }
}

/// 暴露底层 I/O 错误，完整性不匹配则没有外部错误来源。
impl Error for ArtifactError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::IntegrityMismatch { .. } => None,
        }
    }
}

/// 持有一次 Execution 临时脚本及其唯一目录所有权。
#[derive(Debug)]
pub struct MaterializedScript {
    /// 本次 Execution 的专属临时目录；清理后为 `None`。
    execution_directory: Option<PathBuf>,
    /// 由 Artifact 创建且不可被调用方替换的固定脚本路径。
    script_path: PathBuf,
    /// 由 RenderedScript 对完整最终字节预先计算、并在落盘后复验的 expected SHA-256。
    expected_hash: [u8; 32],
}

impl MaterializedScript {
    /// 创建唯一临时目录，写入已冻结的最终脚本字节并复验 expected SHA-256。
    pub fn create(rendered: RenderedScript) -> Result<Self, ArtifactError> {
        let root_directory = std::env::temp_dir().join(CMDBOX_TEMP_DIRECTORY_NAME);
        fs::create_dir_all(&root_directory).map_err(|source| ArtifactError::Io {
            operation: ArtifactOperation::CreateRootDirectory,
            path: root_directory.clone(),
            source,
        })?;

        let execution_directory = root_directory.join(Uuid::new_v4().simple().to_string());
        fs::create_dir(&execution_directory).map_err(|source| ArtifactError::Io {
            operation: ArtifactOperation::CreateExecutionDirectory,
            path: execution_directory.clone(),
            source,
        })?;

        match Self::write_script(&execution_directory, &rendered) {
            Ok(script_path) => Ok(Self {
                execution_directory: Some(execution_directory),
                script_path,
                expected_hash: rendered.artifact_hash(),
            }),
            Err(error) => {
                // 创建未完整完成时只清理刚刚生成的唯一目录，不触碰 CmdBox 临时根其他任务。
                let _ = fs::remove_dir_all(&execution_directory);
                Err(error)
            }
        }
    }

    /// 返回由当前 Artifact 唯一拥有的临时脚本绝对路径。
    pub fn script_path(&self) -> &Path {
        &self.script_path
    }

    /// 返回创建时绑定的完整最终字节 SHA-256。
    pub fn artifact_hash(&self) -> [u8; 32] {
        self.expected_hash
    }

    /// 紧邻进程创建前重新读取脚本并与 expected SHA-256 比较。
    pub(crate) fn verify_before_spawn(&self) -> Result<(), ArtifactError> {
        let actual_hash = hash_file(&self.script_path)?;
        if actual_hash != self.expected_hash {
            return Err(ArtifactError::IntegrityMismatch {
                path: self.script_path.clone(),
            });
        }
        Ok(())
    }

    /// 显式删除当前 Artifact 的唯一目录；成功后 Drop 不会重复删除。
    pub fn cleanup(mut self) -> Result<(), ArtifactError> {
        self.remove_execution_directory()
    }

    /// 在已经创建的唯一目录中写入固定脚本，flush 后复验完整字节 Hash 并返回路径。
    fn write_script(
        execution_directory: &Path,
        rendered: &RenderedScript,
    ) -> Result<PathBuf, ArtifactError> {
        let script_path = execution_directory.join(rendered.file_name());
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&script_path)
            .map_err(|source| ArtifactError::Io {
                operation: ArtifactOperation::CreateScriptFile,
                path: script_path.clone(),
                source,
            })?;

        file.write_all(rendered.bytes())
            .and_then(|()| file.flush())
            .map_err(|source| ArtifactError::Io {
                operation: ArtifactOperation::WriteScript,
                path: script_path.clone(),
                source,
            })?;

        let actual_hash = hash_file(&script_path)?;
        if actual_hash != rendered.artifact_hash() {
            return Err(ArtifactError::IntegrityMismatch { path: script_path });
        }
        Ok(script_path)
    }

    /// 删除仍由当前对象持有的 Execution 专属目录。
    fn remove_execution_directory(&mut self) -> Result<(), ArtifactError> {
        let Some(directory) = self.execution_directory.as_ref() else {
            return Ok(());
        };

        match fs::remove_dir_all(directory) {
            Ok(()) => {
                // 只有文件系统确认删除成功后才释放所有权；失败时保留路径供 Drop 尽力重试。
                self.execution_directory = None;
                Ok(())
            }
            Err(source) => Err(ArtifactError::Io {
                operation: ArtifactOperation::RemoveExecutionDirectory,
                path: directory.clone(),
                source,
            }),
        }
    }
}

/// Drop 只做尽力清理，不能在退出或错误展开期间引入 panic。
impl Drop for MaterializedScript {
    fn drop(&mut self) {
        let _ = self.remove_execution_directory();
    }
}

/// 读取一个受管脚本的全部字节并计算 SHA-256。
fn hash_file(path: &Path) -> Result<[u8; 32], ArtifactError> {
    let bytes = fs::read(path).map_err(|source| ArtifactError::Io {
        operation: ArtifactOperation::ReadScript,
        path: path.to_path_buf(),
        source,
    })?;
    Ok(Sha256::digest(bytes).into())
}

#[cfg(test)]
mod tests {
    //! 最终脚本字节、临时脚本完整性与精确清理测试。

    use std::fs;

    use sha2::{Digest, Sha256};

    use super::{ArtifactError, MaterializedScript, RenderedScript, UTF8_BOM};

    /// 验证中文脚本以 UTF-8 BOM 开头，并能通过未篡改的启动前复验。
    #[test]
    fn writes_utf8_bom_script_and_verifies_expected_hash() {
        let script = "Write-Output '中文路径 C:\\测试'";
        let rendered = RenderedScript::windows_powershell(script);
        let expected_hash: [u8; 32] = Sha256::digest(rendered.bytes()).into();

        assert!(rendered.bytes().starts_with(&UTF8_BOM));
        assert_eq!(&rendered.bytes()[UTF8_BOM.len()..], script.as_bytes());
        assert_eq!(rendered.artifact_hash(), expected_hash);

        let artifact = MaterializedScript::create(rendered).expect("应创建临时脚本");

        let bytes = fs::read(artifact.script_path()).expect("应读取临时脚本");
        assert!(bytes.starts_with(&UTF8_BOM));
        assert_eq!(&bytes[UTF8_BOM.len()..], script.as_bytes());
        assert_eq!(artifact.artifact_hash(), expected_hash);
        assert_eq!(
            artifact
                .script_path()
                .file_name()
                .and_then(|name| name.to_str()),
            Some("script.ps1")
        );
        artifact
            .verify_before_spawn()
            .expect("未篡改脚本应通过复验");
    }

    /// 验证创建后的任意字节变化都会在启动前被拒绝。
    #[test]
    fn rejects_script_changed_after_creation() {
        let rendered = RenderedScript::windows_powershell("Write-Output 'safe'");
        let artifact = MaterializedScript::create(rendered).expect("应创建临时脚本");
        fs::write(artifact.script_path(), b"Write-Output 'changed'").expect("测试应能篡改临时脚本");

        assert!(matches!(
            artifact.verify_before_spawn(),
            Err(ArtifactError::IntegrityMismatch { .. })
        ));
    }

    /// 验证显式清理只移除当前 Execution 的唯一目录。
    #[test]
    fn cleanup_removes_only_owned_execution_directory() {
        let rendered = RenderedScript::windows_powershell("Write-Output 'cleanup'");
        let artifact = MaterializedScript::create(rendered).expect("应创建临时脚本");
        let execution_directory = artifact
            .script_path()
            .parent()
            .expect("脚本应位于唯一目录")
            .to_path_buf();
        let root_directory = execution_directory
            .parent()
            .expect("唯一目录应位于 CmdBox 临时根")
            .to_path_buf();

        artifact.cleanup().expect("应清理当前 Artifact");

        assert!(!execution_directory.exists());
        assert!(root_directory.exists());
    }

    /// 验证并行存在的脚本使用不同随机目录，清理一个租约不会影响另一个。
    #[test]
    fn materializes_scripts_in_unique_owned_directories() {
        let first = MaterializedScript::create(RenderedScript::windows_powershell("exit 0"))
            .expect("应创建第一个临时脚本");
        let second = MaterializedScript::create(RenderedScript::windows_powershell("exit 0"))
            .expect("应创建第二个临时脚本");
        let first_directory = first
            .script_path()
            .parent()
            .expect("第一个脚本应位于唯一目录")
            .to_path_buf();
        let second_directory = second
            .script_path()
            .parent()
            .expect("第二个脚本应位于唯一目录")
            .to_path_buf();

        assert_ne!(first_directory, second_directory);
        first.cleanup().expect("应清理第一个脚本目录");
        assert!(!first_directory.exists());
        assert!(second_directory.exists());
        second.cleanup().expect("应清理第二个脚本目录");
        assert!(!second_directory.exists());
    }
}
