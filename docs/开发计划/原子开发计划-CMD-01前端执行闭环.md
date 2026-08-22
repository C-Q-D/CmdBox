# CMD-01 前端执行闭环原子开发计划

## 计划元数据

- 计划 ID：ATOMIC-CMD-01-UI-001
- 类型：atomic-development
- 修订版本：1
- 状态：active
- 父级 ID：CMD-01
- 创建基线：8fc90cb

## 总体计划

### 产品交付单元

`CMD-01`：用户可在真实 CmdBox 桌面界面中运行一个固定、无破坏性的一次性 Windows PowerShell 验收任务，持续看到 stdout/stderr、后端生命周期、Exit Code 与耗时，并可终止整个受管进程树。

### 用户流程与可观察结果

1. 用户打开采用已确认视觉语法的“执行链路验收”Command Workspace。
2. 纯浏览器环境明确显示“需要 Tauri 桌面宿主”，不伪造可运行状态。
3. 真实 Tauri 环境中，用户点击“运行验收任务”，Rust Core 启动固定脚本并立即返回 Execution ID。
4. UI 只根据专属 Channel 事件显示 Started、批量 stdout/stderr 和唯一终态；输出作为不可信纯文本处理。
5. 用户可在运行中点击“终止任务”；界面先进入 Cancelling，只有 Rust 确认 Job 结束后才显示 Cancelled。
6. 自然结束时显示真实 Exit Code、后端耗时和实时输出丢弃字节数；失败时显示结构化错误而不伪造业务成功。

### 关联验收

- `AC-01`：固定 Windows PowerShell 一次性任务可启动、流式输出、自然结束并显示 Exit Code 与耗时。
- `AC-05`：Cancel 与 CmdBox Core 退出终止整个受管进程树。
- `AC-08`：慢消费、Channel 断开和高频输出不反向阻塞外部进程；UI 输出内存与 DOM 有界。

### 明确排除

- 不实现 Typed Parameter、通用 Preview、Command CRUD、History 或数据库。
- 不实现真实文件选择、路径安全或任何删除副作用。
- 不向 React 暴露任意脚本文本、任意可执行文件、任意进程执行、PID 或任意 PID 终止能力。
- 不在本计划解释通用 Outcome Policy；`CMD-03` 前只呈现 Lifecycle、Exit Code 和内部失败事实。
- 不把永久删除界面的动作映射到固定诊断脚本。

### 现状与代码接缝

- `src-tauri/src/execution/session.rs` 已提供 `ExecutionManager::start_fixed_powershell`、有界 `ExecutionEventReceiver` 和唯一终态。
- `src-tauri/src/execution/manager.rs` 已提供按 `ExecutionId` 的整 Job 取消和 Active 快照，不暴露 PID kill。
- `src-tauri/src/execution/output.rs` 已完成 Reader → Aggregator → Batch，并为 Output Fragment 分配顺序。
- `src-tauri/src/lib.rs` 尚未注册业务 IPC，也尚未管理共享 `ExecutionManager`。
- `src/features/command-workspace/CommandWorkspacePrototype.tsx` 是已确认视觉语法的无副作用原型，尚未接入真实后端。
- 项目验证入口为 `pnpm check:fast`、`pnpm check`、`dev.cmd`、`restart.cmd` 与 `stop.cmd`。

### 跨原子约束

1. Rust Core 始终是进程、脚本、取消和生命周期的信任边界。
2. IPC 只暴露“启动固定验收任务”和“按 Execution ID 取消”两个窄业务入口。
3. Started、每个 Output Batch 与唯一终态都携带同一 Execution ID 和单调递增的事件级 `sequence`；现有片段顺序使用 `fragmentSequence` 区分。
4. Channel 发送失败或消费者变慢只能影响当前 UI Delivery，不能取消 Session、阻塞 Process Reader 或改变外部结果。
5. React 不从按钮点击、计时器、输出文本或 Exit Code 猜测后端 Lifecycle。
6. Output 使用独立有界 Chunk Buffer，只通过 React 文本节点渲染，不 linkify、不解释 HTML 或 ANSI/OSC。
7. 固定脚本必须无文件删除等破坏性行为，但应创建带固定诊断标记、寿命长于根循环的子 PowerShell，供真实宿主验证 Job 整树清理。
8. 每个原子独立验证、提交并立即推送 `origin/master`；提交前只暂存当前原子文件。

### 原子顺序

| 原子 | 唯一结果 | 依赖 | 风险 |
|---|---|---|---|
| CMD01-IPC-01 | Rust 暴露固定任务的窄 Typed IPC | 后端执行内核 | L3 |
| CMD01-IPC-02 | TypeScript 通过类型化网关消费 IPC | CMD01-IPC-01 | L2 |
| CMD01-UI-01 | React 工作区呈现真实执行与取消状态 | CMD01-IPC-02 | L2 |
| CMD01-HOST-01 | 真实 Windows/Tauri 闭环取得验收证据 | CMD01-UI-01 | L3 |

### 执行模式与整体验证

- 执行模式：连续执行。
- Git 策略：规划和每个原子分别提交；验证通过后立即推送当前 `master` 到上游 `origin/master`。
- 基线结果：`pnpm check` 通过；前端 4 项、Rust 单元 19 项、Windows 集成 1 项全部通过。
- 整体回归：`pnpm check`。
- 真实宿主：使用 `dev.cmd` 启动，按 `CMD01-HOST-01` 场景验收；结束后使用 `stop.cmd` 精确停止并清理诊断进程。
- 计划质疑：跨前后端公共契约、并发取消和进程树清理按 L3 评审；计划第二轮结果为 `PASS`。

## CMD01-IPC-01 Rust 暴露固定任务的窄 Typed IPC

- 状态：done
- 支持的验收场景：AC-01、AC-05、AC-08 的 Rust/Tauri 接缝。
- 唯一目标：让 Tauri 调用方只能启动固定验收任务并按 Execution ID 请求整树取消。
- 当前行为与目标行为：当前 `lib.rs` 没有业务命令；完成后注册 `start_fixed_execution` 和 `cancel_execution`，并共享一个 `ExecutionManager`。
- 前置条件与依赖：现有后端执行内核测试通过；计划级 DDD 为 `PASS`。
- 代码定位依据：新增 `src-tauri/src/ipc/execution.rs` 与模块入口；复用 `execution/session.rs`、`execution/manager.rs`；在 `src-tauri/src/lib.rs` 注册 State 和 Commands。
- 允许修改：Rust IPC 模块、`lib.rs`、必要的 serde 依赖和窄契约测试。
- 明确不修改：Execution Core 的 Job/Reader 算法、模板与 Preview、文件系统删除、数据库、任意进程入口。
- 实现步骤：
  1. 定义 camelCase、tagged 的可序列化事件、响应和 `ApiError`。
  2. 固定脚本周期输出 stdout/stderr、HTML/ANSI/URL 测试文本，并创建带固定命令行标记的长寿命子 PowerShell。
  3. 启动 Session 后由独立转发线程消费接收端；为所有事件分配单调事件级 `sequence`，保留 fragment 顺序为 `fragmentSequence`。
  4. Channel 发送失败时结束转发，不请求取消；外部任务仍由 Manager 持有并自然结束。
  5. Cancel 只解析 UUID 并调用共享 Manager；重复请求返回稳定 `accepted/state`。
- 接口、数据与错误契约：`start_fixed_execution({ onEvent }) → { executionId }`；`cancel_execution({ executionId }) → { accepted, state }`；错误至少区分无效 ID、启动失败和取消失败。
- 边界与异常：启动失败不留下 Active；Channel 失败不改变任务；Started→Output→终态只有一个终态，事件级 sequence 严格递增。
- 测试要求：事件映射字段、全事件 sequence、唯一终态、无效 UUID、重复取消、Channel 失败不触发取消。
- 验证命令：`cargo fmt --manifest-path src-tauri/Cargo.toml --check`；`cargo test --manifest-path src-tauri/Cargo.toml ipc::execution`；`cargo test --manifest-path src-tauri/Cargo.toml`。
- 预期结果：两个窄命令可编译注册，新增契约测试通过，原有 Rust 测试不回归。
- 完成判定：不存在任意脚本/PID IPC，固定 Session 可由 Channel 观察并由 Execution ID 取消。
- 交付给下一原子的输出：稳定命令名、请求/响应和事件 discriminated union。
- 停止或重新规划条件：Tauri Channel 无法在不改变 Session 所有权的情况下转发；需要扩大为任意执行能力。
- 风险等级：L3
- DDD 门禁：提交前审查完整 Rust diff、公共 IPC、安全边界和并发语义，必须 `PASS`。
- 计划提交信息：`feat(ipc): [CMD01-IPC-01] 暴露固定任务执行通道`

### 执行记录

- 实际验证：`cargo test --manifest-path src-tauri/Cargo.toml ipc::execution` 5 项通过；完整 Rust 单元测试 24 项和 Windows 集成测试 1 项通过；`cargo fmt --check` 与严格 Clippy 通过；提交前 L3 DDD 第二轮复审为 `PASS`。

## CMD01-IPC-02 TypeScript 通过类型化网关消费 IPC

- 状态：pending
- 支持的验收场景：React 可调用固定任务并消费专属有序事件。
- 唯一目标：提供一个可测试、无任意命令能力的 TypeScript IPC Gateway。
- 当前行为与目标行为：前端未依赖 Tauri JS API；完成后只有 Gateway 知道 Command 名和 `Channel` 构造。
- 前置条件与依赖：CMD01-IPC-01 已提交；使用与 Tauri 2.11 兼容的 `@tauri-apps/api`。
- 代码定位依据：新增 `src/features/command-workspace/execution-gateway.ts` 及测试；依赖入口为 `@tauri-apps/api/core`。
- 允许修改：Gateway、Gateway 测试、`package.json` 和 `pnpm-lock.yaml`。
- 明确不修改：React 视觉组件、Rust IPC、全局 Store、任意 Shell/文件 API。
- 实现步骤：
  1. 定义与 Rust JSON 契约一一对应的 TypeScript 类型。
  2. 封装 Channel 创建、`invoke` 调用和未知错误到 `ApiError` 的归一化。
  3. 通过注入最小 Transport 让单元测试无需伪造全局 Tauri 对象。
- 接口、数据与错误契约：Gateway 只提供 `startFixedExecution(onEvent)` 与 `cancelExecution(executionId)`；不得接受脚本、路径、PID 或可执行文件。
- 边界与异常：命令拒绝、非对象错误和未知错误都形成稳定前端错误；事件类型保持 event/data/sequence 契约。
- 测试要求：命令名与参数键、Channel 回调转发、响应返回、错误归一化、Gateway 类型约束。
- 验证命令：`pnpm test -- execution-gateway`；`pnpm typecheck`。
- 预期结果：Gateway 测试与类型检查通过，业务组件无需直接 import Tauri API。
- 完成判定：前端具备唯一、窄且可替换测试 Transport 的真实 IPC 入口。
- 交付给下一原子的输出：React 可注入的 `FixedExecutionGateway`。
- 停止或重新规划条件：当前 Tauri API 版本与 Rust Channel 契约不兼容。
- 风险等级：L2
- DDD 门禁：提交前审查 TS/Rust 字段一致性与能力最小化，必须 `PASS`。
- 计划提交信息：`feat(frontend): [CMD01-IPC-02] 增加固定任务 IPC 网关`

## CMD01-UI-01 React 工作区呈现真实执行与取消状态

- 状态：pending
- 支持的验收场景：用户从统一 Workspace 启动、观察并取消固定任务。
- 唯一目标：把已确认视觉语法落成由真实后端事件驱动的固定任务工作区。
- 当前行为与目标行为：当前选中项是无副作用永久删除原型；完成后默认选中“执行链路验收”，永久删除条目不获得真实动作，运行区呈现真实生命周期、输出和结果。
- 前置条件与依赖：CMD01-IPC-02 已提交；不得把诊断任务伪装成删除行为。
- 代码定位依据：重构 `CommandWorkspacePrototype.tsx` 为正式 Workspace；新增独立 Output Buffer/Viewer 或 Hook；更新 `App.test.tsx` 和现有 CSS。
- 允许修改：Command Workspace 前端组件、局部状态/Buffer、CSS 和对应测试。
- 明确不修改：真实删除界面能力、参数表单泛化、History、数据库、Rust 进程逻辑。
- 实现步骤：
  1. 用固定无参数 Command Header、Preview 事实和普通风险动作替换选中的删除原型内容。
  2. 纯浏览器 Transport 不可用时显示明确桌面宿主提示并禁用 Run。
  3. 点击 Run 后等待后端事件；只根据 Started/终态推进生命周期，运行中只允许 Cancel。
  4. 独立保存最近 512 KiB Output Chunk，超过上限从最旧 Chunk 裁剪并显示丢弃提示。
  5. 使用普通 React 文本节点显示 stdout/stderr，不生成链接、不执行 HTML、不解释 ANSI/OSC。
  6. 按当前 Execution ID 与 sequence 接收事件，忽略旧任务或重复/倒序事件。
- 接口、数据与错误契约：前端阶段与后端事件映射清晰；Finished 显示 Exit Code、duration 与 dropped bytes；Cancelled/Failed 不伪造成 Success。
- 边界与异常：重复 Run/Cancel 禁用；启动或取消失败可恢复显示；无输出仍显示生命周期；旧 Execution 事件不污染当前结果。
- 测试要求：宿主不可用、启动、输出纯文本、自然终态、取消、重复点击、错误、过期/倒序事件、有界裁剪。
- 验证命令：`pnpm test -- App`；`pnpm check:web`。
- 预期结果：前端测试、类型检查和构建通过；不存在 `dangerouslySetInnerHTML`、自动 linkify 或任意 Tauri invoke。
- 完成判定：真实 Workspace 的用户可观察状态完全由注入 Gateway 的后端事实驱动。
- 交付给下一原子的输出：可在真实 Tauri 宿主操作的固定执行 UI。
- 停止或重新规划条件：已确认视觉语法无法容纳无参数执行、Output 或终态而需要新产品设计。
- 风险等级：L2
- DDD 门禁：提交前审查生命周期映射、输出不可信边界和旧事件隔离，必须 `PASS`。
- 计划提交信息：`feat(ui): [CMD01-UI-01] 接通固定任务执行工作区`

## CMD01-HOST-01 真实 Windows/Tauri 闭环取得验收证据

- 状态：pending
- 支持的验收场景：CMD-01 的完整真实宿主场景与 AC-01、AC-05、AC-08 当前归属。
- 唯一目标：证明 React → Typed IPC → Rust Core → Windows Process 的固定任务闭环满足 CMD-01。
- 当前行为与目标行为：自动测试不能替代真实宿主；完成后每个要求场景都有可复现证据，否则父级不标完成。
- 前置条件与依赖：CMD01-UI-01 已提交；开发入口和浏览器调试规则可用。
- 代码定位依据：`dev.cmd`、`restart.cmd`、`stop.cmd`、应用 UI、Windows 进程表；证据写回 `docs/testing/测试与验收.md`、工作台、阶段记录和本计划。
- 允许修改：验收文档、项目状态文档、计划状态、父级产品拆分状态；仅在验收暴露当前原子内缺陷时修改对应最小代码与测试。
- 明确不修改：CMD-02 及以后能力、真实用户文件、永久删除、发布配置。
- 实现步骤：
  1. 在真实 Tauri 窗口验证自然结束、stdout/stderr、Exit Code 与耗时。
  2. 运行中 Cancel，并从系统进程表确认带诊断标记的子进程消失。
  3. 在自然终止边缘 Cancel，确认只有一个后端终态。
  4. 暂停或显著放慢前端消费，确认外部任务自然完成且 Manager 无 Active 遗留。
  5. 刷新或销毁 WebView 断开 Channel，确认 Rust 任务继续自然完成。
  6. 任务运行中强制结束 Tauri Core，确认诊断子进程消失。
  7. 确认 HTML、ANSI/OSC 与 URL 只显示为惰性文本。
  8. 清理验收进程与临时状态，运行完整回归并同步文档状态。
- 接口、数据与错误契约：不新增产品接口；验收只观察现有固定任务和系统最终状态。
- 边界与异常：任一场景没有真实证据时保持本原子或父级未完成；不得用底层单测替代。
- 测试要求：上述七个真实场景；完整 `pnpm check`；仓库敏感信息与生成物检查。
- 验证命令：`dev.cmd`；按场景操作与进程检查；`stop.cmd`；`pnpm check`；`git status --short --branch`。
- 预期结果：真实宿主所有场景通过，开发进程和诊断子进程均已清理，完整回归通过。
- 完成判定：验收文档记录可复现场景和结果；计划、`CMD-01`、工作台与阶段记录状态与事实一致。
- 交付给下一原子的输出：`CMD-02` 可依赖的真实 Typed IPC、执行 UI 和宿主证据。
- 停止或重新规划条件：无法取得 Tauri 宿主、进程检查或调试能力；发现 Core/Channel 接缝破坏进程所有权。
- 风险等级：L3
- DDD 门禁：提交前审查真实证据、状态声明和任何缺陷修复 diff，必须 `PASS`。
- 计划提交信息：`test(host): [CMD01-HOST-01] 验收固定任务执行闭环`
