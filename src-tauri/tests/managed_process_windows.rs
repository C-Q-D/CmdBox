//! Windows Core 强制退出后的 Job Object 整树清理集成测试。
//!
//! 测试在独立 helper 进程中持有 Job Handle，再由外部测试进程强制结束 helper，最终通过
//! Win32 同步句柄确认 PowerShell 根进程与 CMD 子进程都已经退出。

#![cfg(all(windows, feature = "process-test-helper"))]

use std::fs;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject};

/// `OpenProcess` 仅等待进程退出所需的标准访问权限。
const PROCESS_SYNCHRONIZE_ACCESS: u32 = 0x0010_0000;

/// 等待 helper 写出就绪文件。
fn wait_until_ready(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if path.is_file() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("helper 未按期就绪：{}", path.display());
}

/// 读取 helper 写出的单个 PID。
fn read_pid(path: &Path) -> u32 {
    fs::read_to_string(path)
        .expect("应读取 PID 文件")
        .trim()
        .parse()
        .expect("PID 应为 u32")
}

/// 读取 helper 回报的随机 Artifact 目录名，并限定为 UUID simple 的 32 位十六进制。
fn read_artifact_directory_name(path: &Path) -> String {
    let name = fs::read_to_string(path).expect("应读取 Artifact 目录名");
    assert_eq!(name.len(), 32, "Artifact 目录名长度应固定");
    assert!(
        name.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Artifact 目录名只能包含十六进制字符"
    );
    name
}

/// 等待指定 PID 退出；无法打开句柄表示它已经退出或不可访问，也满足本测试目的。
fn wait_until_process_exits(pid: u32) {
    // SAFETY: 只请求 SYNCHRONIZE，不修改目标进程；成功句柄在本函数中关闭一次。
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE_ACCESS, 0, pid) };
    if handle.is_null() {
        return;
    }
    // SAFETY: Handle 有效，等待后无论结果都关闭。
    let wait_result = unsafe { WaitForSingleObject(handle, 10_000) };
    unsafe {
        CloseHandle(handle);
    }
    assert_ne!(wait_result, WAIT_TIMEOUT, "PID {pid} 未按期退出");
    assert_eq!(wait_result, WAIT_OBJECT_0, "PID {pid} 等待失败");
}

/// 验证 Rust Core helper 被强制结束时，KILL_ON_JOB_CLOSE 清理全部受管子孙进程。
#[test]
fn killing_core_helper_stops_managed_process_tree() {
    let control_directory =
        std::env::temp_dir().join(format!("cmdbox-core-exit-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&control_directory).expect("应创建唯一测试控制目录");
    let helper_path = env!("CARGO_BIN_EXE_cmdbox-job-test-helper");
    let mut helper = Command::new(helper_path)
        .arg(&control_directory)
        .spawn()
        .expect("应启动 Core helper");

    wait_until_ready(&control_directory.join("ready"));
    let root_pid = read_pid(&control_directory.join("root.pid"));
    let child_pid = read_pid(&control_directory.join("child.pid"));
    let artifact_directory_name =
        read_artifact_directory_name(&control_directory.join("artifact-directory.name"));
    let artifact_root = std::env::temp_dir().join("CmdBox");
    let artifact_directory = artifact_root.join(artifact_directory_name);
    assert_eq!(artifact_directory.parent(), Some(artifact_root.as_path()));

    helper.kill().expect("应强制结束 Core helper");
    helper.wait().expect("应回收 Core helper");
    wait_until_process_exits(root_pid);
    wait_until_process_exits(child_pid);

    fs::remove_dir_all(&artifact_directory).expect("应清理强退 helper 无法 RAII 回收的 Artifact");
    assert!(!artifact_directory.exists(), "测试不得遗留受管 Artifact");
    fs::remove_dir_all(&control_directory).expect("应清理唯一测试控制目录");
}
