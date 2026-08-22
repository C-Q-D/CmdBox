//! CmdBox Windows 桌面进程入口。
//!
//! Release 构建隐藏额外控制台窗口，实际应用装配统一委托给 `cmdbox_lib`。

// Release 构建不创建额外的 Windows 控制台窗口。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// 启动 CmdBox 桌面应用。
fn main() {
    cmdbox_lib::run()
}
