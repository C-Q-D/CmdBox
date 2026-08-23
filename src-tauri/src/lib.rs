//! CmdBox Tauri 应用库入口。
//!
//! 应用入口装配无状态 Planner 与共享 Execution Manager，并且只注册 Command Block 的窄
//! 业务命令。IPC 不暴露任意脚本、可执行文件、工作目录、环境、PID 或进程终止能力。

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
        .manage(execution::planner::ExecutionPlanner::new())
        .manage(execution::manager::ExecutionManager::new())
        .invoke_handler(tauri::generate_handler![
            ipc::execution::list_command_blocks,
            ipc::execution::get_command_block,
            ipc::execution::preview_command_block,
            ipc::execution::run_command_block,
            ipc::execution::cancel_execution
        ])
        .run(tauri::generate_context!())
        .expect("CmdBox Tauri 应用启动失败");
}
