//! CmdBox Tauri 构建脚本。
//!
//! 该入口只调用 Tauri 官方构建步骤，用于生成平台资源和编译期上下文。

/// 执行 Tauri 官方构建流程。
fn main() {
    tauri_build::build()
}
