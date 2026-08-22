//! CmdBox 本地进程模块。
//!
//! 当前模块只暴露 Windows 平台的确定性 Runner；进程创建、Job Object 和输出管理将在后续
//! 原子中沿同一边界接入。

/// Windows 平台进程实现。
pub mod windows;
