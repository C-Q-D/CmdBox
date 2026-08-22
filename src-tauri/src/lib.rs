//! CmdBox Tauri 应用库入口。
//!
//! 当前基线只负责创建本地窗口，不暴露任何业务 IPC 或系统权限。后续能力必须在对应产品切片中
//! 通过明确的 Rust 命令和 Capability 接入。

/// Windows 本地进程与 Runner 的后端能力。
#[cfg(windows)]
pub mod process;

/// 创建并运行 CmdBox Tauri 应用。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("CmdBox Tauri 应用启动失败");
}
