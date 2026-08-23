//! CmdBox 一次性任务执行模块。
//!
//! 当前模块负责固定 Built-in Definition、类型化参数、受限模板、可信 Preview，以及临时
//! PowerShell Artifact、输出 Drain 和 Execution Session。全部可执行资源所有权都由 Rust
//! Core 管理；Planner 只通过内部 Serializer/Spec 做纯计算，不注册 Tauri command 或前端调用链。

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

/// Command Block Definition、可信 Preview 与 Run 复验的唯一业务入口。
pub mod planner;

/// 只消费 Planner 授权值的后端 Execution Session。
pub mod session;

/// Windows PowerShell 字面量序列化与 Template AST 渲染，仅供 Planner 使用。
mod serializer;

/// Canonical Execution Spec 的确定性二进制编码与 Hash，仅供 Planner 使用。
mod spec;

/// value/if/each 受限模板 Parser、Validator 与 AST。
pub mod template;
