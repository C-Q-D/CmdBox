//! CmdBox 本地进程模块。
//!
//! 当前模块暴露 Windows 平台的确定性 Runner、挂起进程创建和 Job Object 生命周期能力。

/// Windows 平台进程实现。
pub mod windows;
