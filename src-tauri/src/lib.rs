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

/// Rust serde 公开 DTO 到 TypeScript 的测试期生成与漂移检查。
#[cfg(test)]
mod typescript_contract;

/// 创建并运行 CmdBox Tauri 应用。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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

#[cfg(test)]
mod capability_tests {
    //! 桌面目录选择插件的最小 Capability 回归测试。

    use serde_json::Value;

    /// 验证 Dialog 只开放 Open，且没有引入 default、Save、Message 或文件系统旁路。
    #[test]
    fn dialog_capability_only_allows_open() {
        let capability: Value = serde_json::from_str(include_str!("../capabilities/default.json"))
            .expect("默认 Capability 应为有效 JSON");
        let permissions = capability["permissions"]
            .as_array()
            .expect("默认 Capability 应声明 permissions")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();

        assert!(permissions.contains(&"dialog:allow-open"));
        assert_eq!(
            permissions
                .iter()
                .filter(|permission| permission.starts_with("dialog:"))
                .copied()
                .collect::<Vec<_>>(),
            vec!["dialog:allow-open"]
        );
        for forbidden_prefix in ["fs:", "shell:", "opener:"] {
            assert!(
                permissions
                    .iter()
                    .all(|permission| !permission.starts_with(forbidden_prefix)),
                "不得开放 {forbidden_prefix} 权限"
            );
        }
    }
}
