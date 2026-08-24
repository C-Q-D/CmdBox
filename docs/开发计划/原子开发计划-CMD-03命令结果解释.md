# CMD-03 命令结果解释原子开发计划

## 计划元数据

- 计划 ID：ATOMIC-CMD-03-001
- 类型：atomic-development
- 修订版本：1
- 状态：active
- 父级 ID：CMD-03
- 创建基线：0f58e09

## 总体计划

- 用户结果：用户在统一 Command Workspace 中看到由 Rust Core 按 Command Block 契约生成的业务 Outcome，并能清楚区分任务 Lifecycle、原始 Exit Code 与命令结果。
- 最终用户流程：选择无破坏的普通或特殊退出码验证 Block → Preview → Run → 查看输出 → 在结果卡中分别查看 Lifecycle、Outcome、Exit Code 与耗时；短等待任务可取消，取消不被误报为命令失败。
- 当前行为：Execution 只发布 `Finished`、`Cancelled`、`Failed` 终态与原始 Exit Code；React 明确不推断 Outcome，也没有稳定 Outcome 字段可展示。
- 实现边界：完成普通 Exit Code、特殊 Exit Code、Cancel 和内部失败的结果解释；建立类型化目标结果的聚合模型和纯 Rust 规则测试。真实删除 Executor 产生目标事实、`Partial Failure` 的真实 Tauri 呈现及 AC-06 目标级证据归 `CMD-04`。
- 深模块 Interface：`OutcomePolicy` 由 Rust Definition 持有并进入 Canonical Execution Spec；`ExecutionManager` 只在自然完成后用已验证 Policy 解释结果；IPC 只发布稳定 `Outcome`，不发布 Policy；React 只映射枚举文案和视觉状态。
- 生命周期不变量：Lifecycle 与 Outcome 独立；自然 `Finished` 保留原始 Exit Code 并携带解释后的 Outcome；`Cancelled` 与 Core 内部 `Failed` 的 Outcome 固定为 `none`。
- Preview 不变量：Policy version 必须来自当前 Definition，进入完整 `executionSpecHash`；Run 继续重新读取 Definition、渲染、校验并比较 Hash。
- 目标结果不变量：目标事实必须由可信 Executor 以类型化值提供，永不解析 stdout；全成功为 `success`，全失败为 `failure`，成功/失败混合为 `partialFailure`，空集合或含未知状态为 `none`。
- 安全测试边界：真实执行只允许文本回显、受控 `exit 0/1/3/8/9` 和最多约 5 秒等待；禁止删除、覆盖、移动、安装、网络、注册表、系统配置或其他高影响命令。
- Registry 边界：默认构建始终只有两个正式 Echo Built-in；普通非零、特殊退出码和短等待验证 Block 只在 `ui-validation` feature 下存在。
- 整体回归：每个原子执行窄测试；最后执行 `pnpm check`、feature 测试、真实 Windows/Tauri 安全用户流程、响应式/可访问性检查和危险命令审计。
- Git：每个原子独立验证、提交，并立即推送当前 `master` 到 `origin/master`。
- 计划质疑：首轮 L3 复核发现目标级真实 Tauri 证据与 `CMD-04` 重叠；修订父级边界、明确不伪造目标事实后，第 2 轮限定复核为 `PASS`。

## 原子依赖

```text
CMD03-POLICY-01 → CMD03-SESSION-01 → CMD03-CONTRACT-01 → CMD03-UI-01 → CMD03-HOST-01
```

## CMD03-POLICY-01 建立命令结果策略

- 状态：pending
- 支持的验收场景：普通命令、特殊退出码工具和类型化目标事实都由 Rust 纯规则得到确定 Outcome。
- 唯一目标：建立可验证、版本化且属于 Command Block Definition 的 `OutcomePolicy` 深模块。
- 当前行为与目标行为：Canonical Spec 只有硬编码的 policy version；完成后每个 Definition 持有合法 Policy，Planner 使用真实版本绑定 Preview/Run Hash。
- 前置条件与依赖：CMD-02 已完成；无本计划内依赖。
- 代码定位依据：`src-tauri/src/execution/command.rs`、`planner.rs`、`spec.rs`、`mod.rs`，新增 `outcome.rs` 及直接测试。
- 允许修改：Outcome 纯模型、Definition 内部字段、验证专用 Built-in、Planner/Spec 及其测试。
- 明确不修改：Session 终态、IPC、React、真实删除 Executor、stdout 解析和持久化。
- 实现步骤：定义 serde camelCase `Outcome`；建立 Standard Exit Code、区间型特殊 Exit Code 与 Target Results Policy；拒绝 version 0、非法区间和重叠区间；为正式 Built-in 使用标准策略，为 feature-only 验证提供普通非零和参数化特殊 Exit Code Definition；让 Planner 校验 Policy 并把实际版本写入 Canonical Spec。
- 接口、数据与错误契约：Policy 只在 Rust 内部；非法 Built-in 配置收敛为稳定 `INTERNAL_CONTRACT`，不回显用户值；Outcome wire 值为 `none | success | warning | partialFailure | failure`。
- 边界与异常：未知目标事实保守返回 `none`；目标事实为空不臆测成功；Policy version 改变必须使旧 Hash 失效。
- 测试要求：普通 0/非零、特殊 success/warning/failure 区间、目标全成功/全失败/混合/空/未知、非法版本/区间/重叠、默认 2 个与 feature 5 个 Definition、Policy version Hash 变化。
- 验证命令：Outcome/Command/Planner 窄测、完整 Rust 单测、format、strict Clippy。
- 预期结果：相同 Policy 与类型化事实总是产生相同 Outcome，Policy 变化受 Preview Hash 保护。
- 完成判定：纯计算和 Planner 测试通过，不创建线程、文件或进程。
- 交付给下一原子的输出：经验证并绑定 Hash 的内部 `OutcomePolicy` 和稳定 `Outcome` 类型。
- 停止或重新规划条件：必须通过 stdout 推断结果，或 Policy 无法进入唯一 Preview/Run 计算路径。
- 风险等级：L2
- DDD 门禁：提交前一轮限定范围复核必须 `PASS`。
- 计划提交信息：`feat(core): [CMD03-POLICY-01] 建立命令结果策略`

## CMD03-SESSION-01 生成执行业务结果

- 状态：pending
- 支持的验收场景：普通与特殊退出码在自然完成后由 Rust 生成 Outcome，取消和内部失败保持 `none`。
- 唯一目标：让 Session Supervisor 在唯一终态中生成独立于 Lifecycle 的业务结果。
- 当前行为与目标行为：Session 只保留 Exit Code；完成后私有 `VerifiedExecution` 携带 Policy，自然完成解释 Outcome，其他终态明确为 `none`。
- 前置条件与依赖：`CMD03-POLICY-01`。
- 代码定位依据：`src-tauri/src/execution/planner.rs`、`session.rs`、`ipc/execution.rs` 及直接/Windows 测试。
- 允许修改：Verified 值、Session 内部事件和测试；IPC 映射可暂时显式忽略新增内部字段。
- 明确不修改：公开 IPC wire、React、目标 Executor、Output/Cancel 算法和进程启动参数。
- 实现步骤：让字段私有的 Verified 值携带已校验 Policy；仅在进程自然退出且 Exit Code 完整可用时解释；向内部三种终态加入 Outcome；保持 sequence、唯一终态、清理和 Channel 断开语义。
- 接口、数据与错误契约：`Finished` 保留原始 Exit Code；`Cancelled`/内部 `Failed` 为 `Outcome::None`；本原子不把新字段发布到前端。
- 边界与异常：取消与自然完成竞态继续只允许一个终态；读输出失败、等待失败或内部错误不能伪装成业务 failure。
- 测试要求：exit 0/9/1/3/8、Cancel、内部失败、唯一终态、Active 清理、Channel 断开与既有 Output 回归。
- 验证命令：Session/Planner 窄测、完整 Rust 单测、Windows 集成、format、strict Clippy。
- 预期结果：Rust 内部已拥有完整且不混淆的 Lifecycle + Outcome 终态事实。
- 完成判定：所有终态路径明确携带 Outcome，既有执行与取消不变量不回归。
- 交付给下一原子的输出：可安全映射到公开 DTO 的内部终态 Outcome。
- 停止或重新规划条件：解释结果必须改变进程 Exit Code、取消归属或现有唯一终态算法。
- 风险等级：L3
- DDD 门禁：提交前一轮限定范围复核必须 `PASS`。
- 计划提交信息：`feat(core): [CMD03-SESSION-01] 生成执行业务结果`

## CMD03-CONTRACT-01 发布稳定 Outcome 契约

- 状态：pending
- 支持的验收场景：前端经现有 Run Channel 收到 Rust 生成的稳定 Outcome，不增加任意执行入口。
- 唯一目标：把内部 Outcome 精确发布为 Rust 单一真值生成的 TypeScript Contract。
- 当前行为与目标行为：公开三种终态没有 Outcome；完成后 `Finished`、`Cancelled`、`Failed` 均有必填 Outcome。
- 前置条件与依赖：`CMD03-SESSION-01`。
- 代码定位依据：`src-tauri/src/ipc/execution.rs`、Contract 生成测试、`src/generated/contracts.ts`、前端事件 Fixture。
- 允许修改：终态 DTO、映射、生成契约和为通过类型检查所需的 Fixture 机械更新。
- 明确不修改：五个业务 IPC 命令集合、请求字段、Policy 暴露、Workspace 展示逻辑和权限。
- 实现步骤：给三种终态 DTO 加必填 Outcome；映射内部值；用 `pnpm contract:generate` 更新唯一生成文件；给既有前端终态 Fixture 补显式值，但暂不改变 UI 行为。
- 接口、数据与错误契约：只发布枚举结果；不发布 Policy、脚本、可执行文件、PID、目标事实或底层错误对象。
- 边界与异常：未知 wire 值由 TypeScript 编译/契约漂移门禁暴露；Cancelled/Failed 必须序列化为 `none`。
- 测试要求：Rust serde、IPC 映射、生成漂移、Gateway 五命令白名单、TypeScript typecheck 和既有前端回归。
- 验证命令：Contract 生成/漂移、IPC 窄测、`pnpm typecheck`、前端测试、Rust 回归。
- 预期结果：Rust 与 TypeScript 对五个 Outcome 值及三种终态字段完全一致。
- 完成判定：生成文件无漂移，所有 Fixture 显式、五个业务 IPC 不增不减。
- 交付给下一原子的输出：React 可直接消费且无需推断的 `event.outcome`。
- 停止或重新规划条件：需要增加脚本/PID/Policy 请求或新增任意执行 IPC。
- 风险等级：L3
- DDD 门禁：提交前一轮限定范围复核必须 `PASS`。
- 计划提交信息：`feat(ipc): [CMD03-CONTRACT-01] 发布稳定 Outcome 契约`

## CMD03-UI-01 分离展示生命周期与结果

- 状态：pending
- 支持的验收场景：用户能在现有结果卡中分别辨认自然结束/取消/内部失败与业务 Outcome。
- 唯一目标：让 Workspace 只展示后端 Outcome，并保持既有 `editorial-field-notes` 视觉语法。
- 当前行为与目标行为：结果卡只显示 Lifecycle 式标题；完成后增加固定 Outcome 标签，同时保留 Exit Code、耗时和丢弃字节。
- 前置条件与依赖：`CMD03-CONTRACT-01`。
- 代码定位依据：`src/features/command-workspace/CommandWorkspace.tsx`、`src/app/App.css`、`src/app/App.test.tsx`。
- 允许修改：Workspace 结果状态、枚举到固定中文文案/样式的映射、现有结果卡最小 CSS 和直接测试。
- 明确不修改：页面结构、导航、表单/Preview、Runner、安全说明、结果推断和新视觉原型。
- 实现步骤：保存终态 Outcome；用穷尽映射显示未生成/成功/警告/部分失败/失败；Lifecycle 标题保持独立；对非零 success/warning、取消 none、内部失败 none 建立测试。
- 接口、数据与错误契约：只按枚举选择固定文案和 class；不读取 Exit Code、stdout/stderr、耗时或 message 推断 Outcome；Output 继续按不可信文本渲染。
- 边界与异常：新一轮执行清空旧结果；迟到旧 generation/Execution ID/sequence 事件不得污染当前结果；Outcome `none` 不显示为成功或失败。
- 测试要求：五个枚举、普通非零失败、非零成功/警告、Cancel none、Failed none、竞态、重复终态和不可信文本回归。
- 验证命令：Workspace 窄测、完整前端测试、typecheck、build。
- 预期结果：同一 Exit Code 可按后端策略呈现不同 Outcome，UI 没有业务判断分支。
- 完成判定：结果卡在既有布局中清楚分离两个维度，前端测试证明不推断。
- 交付给下一原子的输出：可进行真实 Tauri 安全验收的完整 Outcome UI。
- 停止或重新规划条件：需要改动已确认的整体界面结构或引入新的产品流程。
- 风险等级：L2
- DDD 门禁：提交前一轮限定范围复核必须 `PASS`。
- 计划提交信息：`feat(ui): [CMD03-UI-01] 分离展示生命周期与结果`

## CMD03-HOST-01 完成结果解释宿主闭环

- 状态：pending
- 支持的验收场景：真实 Windows/Tauri 中普通成功、普通失败、特殊成功/警告/失败和取消均得到准确稳定展示。
- 唯一目标：完成修订后 CMD-03 的真实宿主、安全、文档和发布门禁。
- 当前行为与目标行为：自动测试尚未证明真实 WebView/IPC/Rust Process 全链路；完成后安全验证 Definition 逐条取得预期与实际证据，默认 Registry 恢复为两个正式 Built-in。
- 前置条件与依赖：`CMD03-UI-01`。
- 代码定位依据：`ui-validation` Registry、真实 Tauri 应用、`docs/testing/测试与验收.md`、产品/架构/安全/性能文档、工作台、阶段记录、父级计划和根 `AGENTS.md`。
- 允许修改：真实验收所需测试/脚本、当前权威文档、完成状态与隐私清理后的必要证据。
- 明确不修改：Hero Delete、真实目标 Executor、History、SQLite、持久日志、默认正式 Registry 和发布功能。
- 实现步骤：真实 Tauri 运行默认 success；feature-only 运行普通 exit9 failure、特殊 exit1 success、exit3 warning、exit8 failure、短等待 Cancel none；核对 Lifecycle/Outcome/Exit Code/唯一终态；恢复并验证默认双 Built-in；执行完整门禁、危险命令与敏感信息扫描；记录逐项预期/实际并收口状态。
- 接口、数据与错误契约：真实流程只经过 list/get/preview/run/cancel；不把验证 Definition 带入默认构建；未取得的 console 或宿主证据必须如实标注。
- 边界与异常：只运行受控 exit 和最多约 5 秒等待；不得创建、修改或删除测试目标；终止后确认无活动 Execution 或受管子进程残留。
- 测试要求：`pnpm check`、feature Rust/前端测试、contract drift、format、strict Clippy、真实宿主矩阵、三档响应式/键盘、默认 Registry、diff/危险命令/凭据/私有路径检查。
- 验证命令：项目完整质量门禁与真实 Tauri `ui-validation` 安全流程；不运行 Bundle 重建，除非门禁确有需要。
- 预期结果：CMD-03 的 Exit Code Policy 与 Cancel 从 React → Typed IPC → Rust Core → Windows Process → Channel → React 真实闭环；目标级真实证据明确留给 CMD-04。
- 完成判定：自动和真实宿主证据齐全、默认双 Built-in、文档只描述已实现事实、最终 L3 复核 `PASS`。
- 交付给下一单元的输出：稳定 Outcome Contract、Policy seam 与完成的 CMD-03 验收基线。
- 停止或重新规划条件：真实 Tauri 结果与自动测试不一致、出现进程残留、必须运行危险命令或目标级契约仍与 CMD-04 重叠。
- 风险等级：L3
- DDD 门禁：提交前最终完整 diff 复核必须 `PASS`。
- 计划提交信息：`feat(outcome): [CMD03-HOST-01] 完成结果解释宿主闭环`
