//! CmdBox 一次性任务执行模块。
//!
//! 当前模块负责固定 Built-in Definition、类型化参数与受限模板，以及临时 PowerShell
//! Artifact、输出 Drain 和 Execution Session。全部可执行资源所有权都由 Rust Core 管理；
//! 新增的 Definition/Parameter/Template 模块仅做纯计算，不注册 Tauri command 或前端调用链。

/// 临时 PowerShell Artifact。
pub mod artifact;

/// 固定正常风险参数回显 Command Block Definition。
pub mod command;

/// Active Execution 的短锁索引、状态查询与取消入口。
pub mod manager;

/// stdout/stderr 快速 Drain、增量解码与有界 Batch。
pub mod output;

/// 六类 Parameter Definition、结构化输入校验与确定性规范化。
pub mod parameter;

/// 固定 Windows PowerShell 的后端 Execution Session。
pub mod session;

/// value/if/each 受限模板 Parser、Validator 与 AST。
pub mod template;
