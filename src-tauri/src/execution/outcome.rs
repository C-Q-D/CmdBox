//! Command Block 业务结果的类型化值与版本化解释策略。
//!
//! 本模块只消费可信的 Exit Code 或 Executor 提供的目标事实，不读取或解析 stdout/stderr。
//! Lifecycle 由 Session 单独维护，不能用本模块的 Outcome 替代。

use serde::{Deserialize, Serialize};

/// Rust Core 发布给调用方的稳定业务结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    /// 当前终态没有足够业务事实，或任务没有自然完成。
    None,
    /// Command 契约判定业务目标全部成功。
    Success,
    /// Command 契约判定任务完成但存在需关注的警告。
    Warning,
    /// 多目标任务中同时存在已确认成功与失败。
    PartialFailure,
    /// Command 契约判定业务目标失败。
    Failure,
}

/// Executor 为一个目标提供的可信结果事实。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetOutcomeFact {
    /// 该目标已被 Executor 明确确认成功。
    Success,
    /// 该目标已被 Executor 明确确认失败。
    Failure,
    /// Executor 不能确认该目标最终结果。
    Unknown,
}

/// 包含首尾端点的 Exit Code 区间。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExitCodeRange {
    /// 区间包含的最小 Exit Code。
    pub(crate) start: i32,
    /// 区间包含的最大 Exit Code。
    pub(crate) end: i32,
}

/// 当前 Windows MVP 支持的结果事实来源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub(crate) enum OutcomePolicyKind {
    /// 按显式成功与警告区间解释自然完成的 Exit Code。
    ExitCode {
        /// 被解释为成功的区间。
        success: Vec<ExitCodeRange>,
        /// 被解释为警告的区间。
        warning: Vec<ExitCodeRange>,
    },
    /// 按 Executor 提供的类型化目标事实聚合。
    TargetResults,
}

/// Command Block Definition 持有的版本化结果解释契约。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OutcomePolicy {
    /// Policy 语义变化时递增并进入 Execution Spec Hash 的版本。
    version: u32,
    /// Policy 使用的可信事实类型和解释规则。
    kind: OutcomePolicyKind,
}

/// 固定 Definition 的 Outcome Policy 配置错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutcomePolicyError {
    /// Policy version 必须为非零值。
    ZeroVersion,
    /// Exit Code 区间首端点大于尾端点。
    InvalidRange,
    /// 两个区间存在重叠并造成不明确配置。
    OverlappingRanges,
}

impl OutcomePolicy {
    /// 创建普通命令的稳定策略：只把 Exit Code 0 解释为成功。
    pub(crate) fn standard() -> Self {
        Self::exit_code(1, vec![ExitCodeRange { start: 0, end: 0 }], Vec::new())
    }

    /// 创建显式 Exit Code 区间策略。
    pub(crate) fn exit_code(
        version: u32,
        success: Vec<ExitCodeRange>,
        warning: Vec<ExitCodeRange>,
    ) -> Self {
        Self {
            version,
            kind: OutcomePolicyKind::ExitCode { success, warning },
        }
    }

    /// 创建由真实 Executor 目标事实驱动的策略。
    #[allow(dead_code)]
    pub(crate) fn target_results(version: u32) -> Self {
        Self {
            version,
            kind: OutcomePolicyKind::TargetResults,
        }
    }

    /// 返回进入 Canonical Execution Spec 的语义版本。
    pub(crate) const fn version(&self) -> u32 {
        self.version
    }

    /// 校验固定 Definition 的版本、区间方向和唯一归属。
    pub(crate) fn validate(&self) -> Result<(), OutcomePolicyError> {
        if self.version == 0 {
            return Err(OutcomePolicyError::ZeroVersion);
        }
        let OutcomePolicyKind::ExitCode { success, warning } = &self.kind else {
            return Ok(());
        };
        let ranges = success.iter().chain(warning.iter()).collect::<Vec<_>>();
        if ranges.iter().any(|range| range.start > range.end) {
            return Err(OutcomePolicyError::InvalidRange);
        }
        for (index, range) in ranges.iter().enumerate() {
            if ranges
                .iter()
                .skip(index + 1)
                .any(|other| ranges_overlap(range, other))
            {
                return Err(OutcomePolicyError::OverlappingRanges);
            }
        }
        Ok(())
    }

    /// 只按 Policy 解释自然完成的原始 Exit Code。
    #[allow(dead_code)]
    pub(crate) fn interpret_exit_code(&self, exit_code: i32) -> Outcome {
        let OutcomePolicyKind::ExitCode { success, warning } = &self.kind else {
            return Outcome::None;
        };
        if success.iter().any(|range| range.contains(exit_code)) {
            Outcome::Success
        } else if warning.iter().any(|range| range.contains(exit_code)) {
            Outcome::Warning
        } else {
            Outcome::Failure
        }
    }

    /// 只按类型化目标事实聚合结果；不能确认时返回 `none`。
    #[allow(dead_code)]
    pub(crate) fn interpret_target_results(&self, facts: &[TargetOutcomeFact]) -> Outcome {
        if !matches!(self.kind, OutcomePolicyKind::TargetResults)
            || facts.is_empty()
            || facts.contains(&TargetOutcomeFact::Unknown)
        {
            return Outcome::None;
        }
        let has_success = facts.contains(&TargetOutcomeFact::Success);
        let has_failure = facts.contains(&TargetOutcomeFact::Failure);
        match (has_success, has_failure) {
            (true, true) => Outcome::PartialFailure,
            (true, false) => Outcome::Success,
            (false, true) => Outcome::Failure,
            (false, false) => Outcome::None,
        }
    }
}

impl ExitCodeRange {
    /// 判断一个 Exit Code 是否落在包含端点的区间内。
    #[allow(dead_code)]
    const fn contains(self, exit_code: i32) -> bool {
        self.start <= exit_code && exit_code <= self.end
    }
}

/// 判断两个包含端点的 Exit Code 区间是否共享至少一个值。
const fn ranges_overlap(first: &ExitCodeRange, second: &ExitCodeRange) -> bool {
    first.start <= second.end && second.start <= first.end
}

#[cfg(test)]
mod tests {
    //! Outcome wire 值、Policy 配置和可信事实聚合的纯计算测试。

    use super::{ExitCodeRange, Outcome, OutcomePolicy, OutcomePolicyError, TargetOutcomeFact};

    /// 验证稳定 wire 值不会依赖 Rust 枚举名称格式。
    #[test]
    fn serializes_stable_outcome_wire_values() {
        let values = [
            (Outcome::None, "none"),
            (Outcome::Success, "success"),
            (Outcome::Warning, "warning"),
            (Outcome::PartialFailure, "partialFailure"),
            (Outcome::Failure, "failure"),
        ];

        for (outcome, expected) in values {
            assert_eq!(
                serde_json::to_value(outcome).expect("Outcome 应可序列化"),
                expected
            );
        }
    }

    /// 验证普通命令只把零解释为成功，其他 Exit Code 均为失败。
    #[test]
    fn interprets_standard_exit_codes() {
        let policy = OutcomePolicy::standard();

        assert_eq!(policy.validate(), Ok(()));
        assert_eq!(policy.interpret_exit_code(0), Outcome::Success);
        assert_eq!(policy.interpret_exit_code(1), Outcome::Failure);
        assert_eq!(policy.interpret_exit_code(-1), Outcome::Failure);
    }

    /// 验证特殊工具可以显式声明非零成功与警告区间。
    #[test]
    fn interprets_special_exit_code_ranges() {
        let policy = OutcomePolicy::exit_code(
            3,
            vec![ExitCodeRange { start: 0, end: 1 }],
            vec![ExitCodeRange { start: 2, end: 7 }],
        );

        assert_eq!(policy.validate(), Ok(()));
        assert_eq!(policy.version(), 3);
        assert_eq!(policy.interpret_exit_code(1), Outcome::Success);
        assert_eq!(policy.interpret_exit_code(3), Outcome::Warning);
        assert_eq!(policy.interpret_exit_code(8), Outcome::Failure);
    }

    /// 验证目标事实只在全部事实可确认时生成确定 Outcome。
    #[test]
    fn aggregates_typed_target_facts_conservatively() {
        let policy = OutcomePolicy::target_results(2);

        assert_eq!(
            policy.interpret_target_results(&[TargetOutcomeFact::Success]),
            Outcome::Success
        );
        assert_eq!(
            policy.interpret_target_results(&[TargetOutcomeFact::Failure]),
            Outcome::Failure
        );
        assert_eq!(
            policy.interpret_target_results(&[
                TargetOutcomeFact::Success,
                TargetOutcomeFact::Failure,
            ]),
            Outcome::PartialFailure
        );
        assert_eq!(policy.interpret_target_results(&[]), Outcome::None);
        assert_eq!(
            policy.interpret_target_results(&[
                TargetOutcomeFact::Success,
                TargetOutcomeFact::Unknown,
            ]),
            Outcome::None
        );
        assert_eq!(policy.interpret_exit_code(0), Outcome::None);
    }

    /// 验证版本零、反向区间和任意重叠区间均被拒绝。
    #[test]
    fn rejects_invalid_or_overlapping_policy_configuration() {
        assert_eq!(
            OutcomePolicy::target_results(0).validate(),
            Err(OutcomePolicyError::ZeroVersion)
        );
        assert_eq!(
            OutcomePolicy::exit_code(1, vec![ExitCodeRange { start: 2, end: 1 }], Vec::new(),)
                .validate(),
            Err(OutcomePolicyError::InvalidRange)
        );
        assert_eq!(
            OutcomePolicy::exit_code(
                1,
                vec![
                    ExitCodeRange { start: 0, end: 2 },
                    ExitCodeRange { start: 2, end: 4 },
                ],
                Vec::new(),
            )
            .validate(),
            Err(OutcomePolicyError::OverlappingRanges)
        );
        assert_eq!(
            OutcomePolicy::exit_code(
                1,
                vec![ExitCodeRange { start: 0, end: 2 }],
                vec![ExitCodeRange { start: 2, end: 4 }],
            )
            .validate(),
            Err(OutcomePolicyError::OverlappingRanges)
        );
    }
}
