//! CmdBox 一次性任务执行模块。
//!
//! 当前模块先负责临时 PowerShell Artifact 的创建、完整性复验和清理；进程会话与输出管理
//! 将在后续原子中接入，并继续由 Rust Core 持有资源所有权。

/// 临时 PowerShell Artifact。
pub mod artifact;

/// stdout/stderr 快速 Drain、增量解码与有界 Batch。
pub mod output;
