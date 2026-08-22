//! Windows 平台进程能力入口。
//!
//! 本模块集中管理依赖 Win32 的 Runner、Job Object 与进程句柄，避免上层业务绕过确定性
//! 执行契约直接查找或启动任意程序。

/// Windows PowerShell 5.1 的确定性 Runner。
pub mod runner;
