//! Job Object 的 Core 强退清理测试辅助进程。
//!
//! 本二进制只在 `process-test-helper` feature 下构建。它启动一个会创建子进程的受管
//! PowerShell，写出根/子 PID 后保持 Job Handle；外部集成测试强制结束本进程，以验证
//! `KILL_ON_JOB_CLOSE` 会清理整个受管树。它不进入普通 CmdBox 应用产物。

use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use cmdbox_lib::execution::artifact::{MaterializedScript, RenderedScript};
use cmdbox_lib::process::windows::managed_process::ManagedProcess;
use cmdbox_lib::process::windows::runner::WindowsPowerShellRunner;

/// 启动受管测试树并保持 Job Handle，直到外部测试结束本 helper。
fn main() -> Result<(), Box<dyn Error>> {
    let control_directory = control_directory_from_arguments()?;
    fs::create_dir_all(&control_directory)?;
    let child_pid_file = control_directory.join("child.pid");
    let escaped_child_pid_file = child_pid_file.to_string_lossy().replace('\'', "''");
    let script = format!(
        "$child = Start-Process -FilePath $env:ComSpec -ArgumentList '/d /c ping -t 127.0.0.1' -PassThru; Set-Content -LiteralPath '{escaped_child_pid_file}' -Value $child.Id; Wait-Process -Id $child.Id"
    );
    let runner = WindowsPowerShellRunner::resolve()?;
    let rendered = RenderedScript::windows_powershell(&script);
    let artifact = MaterializedScript::create(rendered)?;
    let launch = runner.process_launch(artifact, &std::env::temp_dir());
    let managed = ManagedProcess::spawn(launch)?;
    fs::write(
        control_directory.join("root.pid"),
        managed.process_id().to_string(),
    )?;
    wait_for_file(&child_pid_file)?;
    fs::write(control_directory.join("ready"), b"ready")?;

    // 保持 `managed` 在作用域中，使 helper 存活期间持有 Job Handle；外部测试会强制结束进程。
    let _managed = managed;
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

/// 从第一个命令行参数读取外部测试创建的控制目录。
fn control_directory_from_arguments() -> Result<PathBuf, Box<dyn Error>> {
    let value = std::env::args_os().nth(1).ok_or("缺少控制目录参数")?;
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err("控制目录必须是绝对路径".into());
    }
    Ok(path)
}

/// 等待受管 PowerShell 写出子进程 PID，避免过早声明 helper 已就绪。
fn wait_for_file(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if path.is_file() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    }
    Err(format!("子进程 PID 文件未按期生成：{}", path.display()).into())
}
