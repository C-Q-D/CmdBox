//! CmdBox Tauri 应用库入口。
//!
//! 应用入口负责装配共享 Execution Manager，并且只注册经过产品切片确认的窄业务命令。
//! 当前 IPC 只能启动固定验收任务或按 Execution ID 取消，不暴露任意进程能力。

/// Windows 本地进程与 Runner 的后端能力。
#[cfg(windows)]
pub mod process;

/// 一次性任务的 Artifact 与后续执行会话能力。
pub mod execution;

/// Tauri 命令、序列化契约与 Rust Core 之间的窄适配层。
pub mod ipc;

/// 创建并运行 CmdBox Tauri 应用。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(execution::manager::ExecutionManager::new())
        .invoke_handler(tauri::generate_handler![
            ipc::execution::start_fixed_execution,
            ipc::execution::cancel_execution
        ])
        .run(tauri::generate_context!())
        .expect("CmdBox Tauri 应用启动失败");
}
