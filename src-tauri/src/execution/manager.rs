//! 运行中 Execution 的短锁索引、状态查询与整树取消入口。
//!
//! Manager 只在 HashMap 插入、查询和移除时持有全局锁；进程等待、输出读取和 Job 操作均
//! 在锁外执行。取消能力只持有当前 Execution 的 Job 句柄，不暴露任意 PID 终止接口。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use uuid::Uuid;

use crate::process::windows::managed_process::{ManagedProcessCancellation, ManagedProcessError};

/// 一次后端 Execution 的稳定标识。
pub type ExecutionId = Uuid;

/// Manager 可观察的非终态生命周期。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveExecutionState {
    /// 进程已经恢复运行，尚未接受取消。
    Running,
    /// 已接受取消并请求终止 Job，等待确认进程树结束。
    Cancelling,
}

/// Active Execution 的只读快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveExecutionSnapshot {
    /// Execution 的稳定 ID。
    pub execution_id: ExecutionId,
    /// 读取快照时观察到的运行状态。
    pub state: ActiveExecutionState,
}

/// 一次取消调用的稳定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CancelResult {
    /// 本次调用是否首次把状态从 Running 推进到 Cancelling。
    pub accepted: bool,
    /// Execution 不存在时为 `None`，存在时返回当前状态。
    pub state: Option<ActiveExecutionState>,
}

/// Manager 内部保存的最小 Active 记录。
pub(crate) struct ActiveExecution {
    /// 可被 Supervisor 与取消调用共同观察的非终态状态。
    pub(crate) state: Arc<Mutex<ExecutionControlState>>,
    /// 只针对当前 Execution Job 的取消入口。
    pub(crate) cancellation: ManagedProcessCancellation,
}

/// 取消入口与 Supervisor 用同一锁裁决终态边界。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutionControlState {
    /// 进程运行中，尚未接受取消。
    Running,
    /// 已接受取消，正在等待受管进程退出。
    Cancelling,
    /// Supervisor 已观察到进程终态，不再接受取消。
    Terminated,
}

/// 可克隆的全局 Execution Manager。
#[derive(Clone, Default)]
pub struct ExecutionManager {
    /// 共享 Active 索引；锁只保护 HashMap 本身。
    inner: Arc<Mutex<HashMap<ExecutionId, Arc<ActiveExecution>>>>,
}

impl ExecutionManager {
    /// 创建空的 Execution Manager。
    pub fn new() -> Self {
        Self::default()
    }

    /// 首次请求取消当前 Execution 的整个 Job；重复调用返回稳定当前状态。
    pub fn cancel(&self, execution_id: ExecutionId) -> Result<CancelResult, ManagedProcessError> {
        let active = {
            let active_map = lock_unpoisoned(&self.inner);
            active_map.get(&execution_id).cloned()
        };
        let Some(active) = active else {
            return Ok(CancelResult {
                accepted: false,
                state: None,
            });
        };

        {
            let mut state = lock_unpoisoned(&active.state);
            match *state {
                ExecutionControlState::Cancelling => {
                    return Ok(CancelResult {
                        accepted: false,
                        state: Some(ActiveExecutionState::Cancelling),
                    });
                }
                ExecutionControlState::Terminated => {
                    return Ok(CancelResult {
                        accepted: false,
                        state: None,
                    });
                }
                ExecutionControlState::Running => {}
            }
            *state = ExecutionControlState::Cancelling;
        }

        if let Err(error) = active.cancellation.terminate_job() {
            let mut state = lock_unpoisoned(&active.state);
            if *state == ExecutionControlState::Cancelling {
                *state = ExecutionControlState::Running;
            }
            return Err(error);
        }

        Ok(CancelResult {
            accepted: true,
            state: Some(ActiveExecutionState::Cancelling),
        })
    }

    /// 返回当前 Active Execution 的瞬时快照，不跨调用持有全局锁。
    pub fn active_snapshot(&self) -> Vec<ActiveExecutionSnapshot> {
        let active_map = lock_unpoisoned(&self.inner);
        active_map
            .iter()
            .filter_map(|(execution_id, active)| {
                let state = match *lock_unpoisoned(&active.state) {
                    ExecutionControlState::Running => ActiveExecutionState::Running,
                    ExecutionControlState::Cancelling => ActiveExecutionState::Cancelling,
                    ExecutionControlState::Terminated => return None,
                };
                Some(ActiveExecutionSnapshot {
                    execution_id: *execution_id,
                    state,
                })
            })
            .collect()
    }

    /// 在 Resume 前登记一个已经绑定事件接收端和取消入口的 Execution。
    pub(crate) fn insert(&self, execution_id: ExecutionId, active: Arc<ActiveExecution>) {
        lock_unpoisoned(&self.inner).insert(execution_id, active);
    }

    /// 终态发布后移除当前 Execution；不影响同 ID 以外的任务。
    pub(crate) fn remove(&self, execution_id: ExecutionId) {
        lock_unpoisoned(&self.inner).remove(&execution_id);
    }
}

/// Poison 只代表另一个线程在持锁时 panic；恢复内部值可保证清理与取消入口仍可工作。
pub(crate) fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
