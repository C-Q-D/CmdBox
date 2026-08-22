//! CmdBox 一次性任务执行模块。
//!
//! 当前模块负责临时 PowerShell Artifact、输出 Drain 以及固定脚本 Execution Session，全部
//! 资源所有权都由 Rust Core 管理；尚不注册 Tauri command 或前端调用链。

/// 临时 PowerShell Artifact。
pub mod artifact;

/// Active Execution 的短锁索引、状态查询与取消入口。
pub mod manager;

/// stdout/stderr 快速 Drain、增量解码与有界 Batch。
pub mod output;

/// 固定 Windows PowerShell 的后端 Execution Session。
pub mod session;
