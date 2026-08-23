//! CmdBox Tauri IPC 模块。
//!
//! 本模块只导出按产品行为命名的窄命令，不提供任意 Shell、文件或 PID 操作入口。

/// Command Block 列表、详情、Preview、Run、事件转发和取消契约。
pub mod execution;
