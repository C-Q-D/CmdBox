# CmdBox MVP 产品拆分

## 计划元数据

- 计划 ID：SCOPE-CMDBOX-001
- 类型：product-scope
- 修订版本：4
- 状态：active
- 创建基线：不适用

## 总体方案

- 目标：交付一个 Windows First 的桌面应用，把常用一次性 CLI 命令变成可参数化、可预览、安全执行并可重复使用的 Command Block。
- 范围：Windows PowerShell/CMD、一次性进程、实时输出、Cancel、Hero Delete、Command Block 持久化、搜索与历史。
- 能力域：确定性执行内核；Typed Template 与 Preview；破坏性路径安全；快速删除；本地持久化；命令发现与历史。
- 关键假设：Windows PowerShell 可作为首个稳定 Runner；首个用户价值由快速删除多个超大文件夹验证；跨平台和交互终端不属于当前 MVP。

### 全局业务不变量

1. 用户看到的 Preview 与实际 Run 必须绑定同一完整 Execution Spec。
2. 破坏性 Built-in Command 在 Run 前必须重新验证参数、路径和目标身份。
3. Cancel 与 CmdBox Core 退出必须终止整个受管进程树。
4. 参数只能通过类型验证和 Shell Serializer 进入脚本。
5. 安全检查不得递归扫描目标目录内容。
6. 外部命令 Outcome 与日志、数据库、UI 的记录结果相互隔离。
7. CmdBox 的观察和记录机制不得显著干扰外部命令性能。

### 跨单元约束

- Rust Core 是进程、模板、安全、数据库和日志的信任边界。
- React 只通过明确 Typed IPC 发起业务操作，不暴露任意进程执行或 PID 终止入口。
- Execution 生命周期事件必须有 Execution ID 和可排序 sequence。
- 大量 Output 采用聚合、Batch、Tauri Channel 和有界内存；CMD-02 当前使用 512 KiB、2048 非空 Chunk 的有界直接渲染，虚拟化 Viewer 留待后续性能单元和发布门禁。
- Windows 文件系统和 Job Object 行为必须在真实 Windows 环境验证。
- 每个声称用户价值的单元都必须通过最小真实 Tauri 垂直路径验收，即从 React UI 经 Typed IPC 到 Rust Core，再观察用户可见结果或系统最终状态；`CMD-08` 只负责体验整合，不能作为此前单元的首次全链路接入。
- 当前实施顺序采用后端先行：可先形成独立验证的 Rust 后端基础，但不得据此把对应产品单元标为 `done`；任何前端产品实现开始前必须先完成界面原型设计并取得用户确认。

### 原始需求覆盖关系

| 来源需求 | 覆盖单元 |
|---|---|
| 不再记忆、查找和手动改写常用命令 | CMD-02、CMD-05、CMD-06、CMD-08 |
| 一次性命令执行、实时输出和结束状态 | CMD-01 |
| 多文件夹参数和安全 Preview | CMD-02、CMD-04 |
| PowerShell/CMD 的真实一次性执行 | CMD-01、CMD-02 |
| 按 Command Outcome Policy 解释结果 | CMD-03 |
| Windows 快速删除超大文件夹 | CMD-04 |
| Cancel 和进程树管理 | CMD-01、CMD-04 |
| Command Block、分类、收藏和参数记忆 | CMD-05、CMD-06 |
| History、日志和再次执行 | CMD-07 |
| 完整产品 UI、搜索和编辑 | CMD-08 |
| 性能与可靠性约束 | 所有单元，重点 CMD-01、CMD-02、CMD-04、CMD-07 |

## 交付顺序

1. `CMD-01`：运行并控制一个固定的一次性 PowerShell 任务。
2. `CMD-02`：通过 Typed Parameter 预览并执行 PowerShell/CMD 命令。
3. `CMD-03`：按 Command Outcome Policy 解释执行结果。
4. `CMD-04`：安全快速删除多个用户选择的文件夹。
5. `CMD-05`：保存并重新打开一个 Command Block。
6. `CMD-06`：从命令库找到并快速再次填写 Command Block。
7. `CMD-07`：查看一次 Execution 的稳定结果与完整历史。
8. `CMD-08`：通过完整桌面工作区管理和运行命令库。

`CMD-05` 的持久化实现准备可与 `CMD-04` 的 Windows 场景验证并行探索，但在产品验收上仍按上述顺序推进，避免在执行核心未证明前扩大 CRUD 和 UI。

# 交付单元卡

## CMD-01 运行并控制一个固定的一次性 PowerShell 任务

- 状态：done
- 用户或业务价值：用户能在 CmdBox 中启动一个非交互 PowerShell 任务，实时看到输出，并在需要时终止整个任务。
- 参与者：Windows 用户。
- 触发与前置条件：CmdBox 已启动；系统存在确定的 Windows PowerShell Runner。
- 预期结果：用户从最小 React 界面经 Typed IPC 启动任务；任务经历 Started、Output、Finished 或 Cancelled；用户看到 Exit Code 和耗时；任务及其子进程受统一 Execution 管理。UI 慢消费或 Channel 断开不阻塞 Process Reader，也不改变已经发生的外部结果。实时 Output 始终按不可信惰性文本显示。
- 验收场景：在真实 Tauri 宿主中运行会持续输出并创建子进程的固定脚本，确认 stdout/stderr 顺序、自然退出、UI 慢消费或断开、Cancel 与自然退出竞态，以及 CmdBox Core 强制退出时不存在遗留受管进程；再输出 HTML、ANSI/OSC 控制序列和 URL，确认界面不解释 HTML、不执行控制效果、不自动 linkify 或打开链接，只显示惰性文本。
- 来源需求或验收条件：一次性命令、stdout/stderr、运行中终止；AC-01、AC-05、AC-08。
- 明确不包含：模板参数、Command CRUD、任意命令编辑、文件夹删除、History UI。
- 依赖：无。
- 风险或假设：Windows Job Object 分配竞态、输出背压、编码和 Runner 定位必须在真实 Windows 环境验证。

## CMD-02 通过 Typed Parameter 预览并执行 PowerShell/CMD 命令

- 状态：done
- 用户或业务价值：用户只填写业务参数即可得到准确、可读且不会因引号错误改变语义的命令 Preview，并以声明的 PowerShell 或 CMD Runner 实际执行。
- 参与者：Windows 用户、Command Block 作者。
- 触发与前置条件：Execution Core 可以运行固定脚本；存在内置 Command Block Draft。
- 预期结果：Text、Number、Boolean、Select、Folder 和 Folders 参数经验证与对应 Shell Serializer 生成 `RenderedExecution`；`executionSpecHash` 覆盖 Command Block ID、revision、Runner、Runner options、完整渲染产物、规范化参数、工作目录、配置环境和安全策略版本。Run 重新读取当前 Command Block 与配置、重新验证、重新渲染并比较完整 Execution Spec Hash，匹配后才通过最小真实 Tauri 路径启动声明的 PowerShell 或 CMD 一次性任务。
- 验收场景：对 PowerShell 和 CMD 分别使用中文、空格、单引号、多路径和条件/循环模板完成 Preview 与真实执行；逐项改变参数、Command revision、Runner、Runner options、工作目录、配置环境或安全策略版本后使用旧 Hash，系统均拒绝启动；CMD 必须在真实 Windows 中通过中文及特殊字符编码验收，未通过前不得宣称 CMD 支持完成。
- 来源需求或验收条件：参数表单、Multi Folder、Preview、escaping；AC-02、AC-03。
- 明确不包含：持久化 Command、用户自定义 Raw Parameter、破坏性路径策略、完整编辑器。
- 依赖：CMD-01。
- 风险或假设：不能把 Preview 展示截断误用为完整执行内容；模板解析与参数序列化必须位于 Rust Core；CMD 编码属于本单元完成门禁而不是以后补做的发布细节。
- 完成证据：九个原子全部完成；前端 7 个文件、91 项测试通过；默认真实 Tauri 中 PowerShell/CMD 均完成六类参数回显、空 stderr、唯一 Finished 与 Exit Code 0；显式 `ui-validation` 短等待完成 UI Cancel 和唯一 Cancelled；三档响应式、单层标题栏、键盘及无危险命令检查通过。本轮独立 WebView console attachment 未取得，真实流程无可见错误提示或 Vite overlay，该限制已在验收清单单列。

## CMD-03 按 Command Outcome Policy 解释执行结果

- 状态：done
- 用户或业务价值：用户看到的是命令契约定义的真实结果，而不是把所有非零 Exit Code 一律误报为失败。
- 参与者：Windows 用户、Command Block 作者。
- 触发与前置条件：一次性 PowerShell/CMD 任务已经产生 Lifecycle、Exit Code、Cancel 原因和必要的命令结果数据。
- 预期结果：Rust Core 根据 Command Block 的 `OutcomePolicy` 生成独立于 Lifecycle 的业务 Outcome；普通命令、特殊退出码工具和取消通过最小真实 Tauri 路径稳定呈现。本单元同时建立类型化目标结果的聚合规则，但不伪造尚不存在的目标 Executor 或目标事实。
- 验收场景：分别运行 Exit Code 0 的普通成功命令、非零普通失败命令、具有非零成功/警告码策略的测试命令，以及取消中的命令；确认 Lifecycle、Exit Code 与业务 Outcome 不被混为一个字段，UI 使用稳定结果而不是自行猜测。
- 来源需求或验收条件：Outcome Policy、Execution State 与 Outcome 分离、Robocopy 特殊退出码；AC-01，以及 AC-06 所需的 Outcome 基础契约。真实目标级 AC-06 证据归 CMD-04。
- 明确不包含：Hero Delete 的路径安全、History 持久化、Robocopy 产品 UI。
- 依赖：CMD-01、CMD-02。
- 风险或假设：不同 Command 的成功语义必须显式配置或由 Built-in 固定，不允许前端按单一规则推断。

## CMD-04 安全快速删除多个用户选择的文件夹

- 状态：in_progress
- 用户或业务价值：用户无需查找和改写命令，即可安全地永久删除多个超大文件夹，并明确知道每个目标的结果。
- 参与者：Windows 用户。
- 触发与前置条件：用户在真实 Tauri 界面选择一个或多个绝对目录并完成 Preview；CMD-01、CMD-02、CMD-03 已通过。
- 预期结果：系统规范化并折叠路径集合，拦截关键路径和不明确 Reparse Point，绑定 Path Fingerprint；Run 二次验证后启动唯一的破坏性 Execution，由真实删除 Executor 产生类型化目标事实，经 CMD-03 的 Policy 聚合并在真实 Tauri 中展示 Success、Partial Failure 或 Failure。应用仍在运行时取消不暗示回滚，目标级结果区分已确认删除、失败、尚未开始和无法确认；Core 强退在本单元只承诺终止整棵进程树，不承诺重启后恢复结果。
- 验收场景：通过真实 Tauri 路径对普通多目录完成删除；对磁盘根目录和系统目录阻断；对 Preview 后被替换的目录拒绝；对部分被占用目标返回 Partial Failure；对同一已确认 `executionSpecHash` 的双击和并发 Run 只允许一个外部删除进程；对 A 已删除、B 处理中、C 未开始时 Cancel 以及接近自然退出的 Cancel 竞态，终止整棵进程树并显示当前可证明的目标级结果；Core 强退场景只验收整树清理，并明确不把退出表达为无副作用或自动回滚。重启后的可恢复目标证据由 `CMD-07` 验收。
- 来源需求或验收条件：Hero Delete、安全评审和性能评审；AC-03、AC-04、AC-05、AC-06、AC-09。
- 明确不包含：回收站恢复、递归安全扫描、删除链接本身的专用命令、复制和移动 UI。
- 依赖：CMD-01、CMD-02、CMD-03。
- 风险或假设：不可逆数据损失；Windows Final Path/File ID/Reparse Point 行为；删除命令真实 Exit 语义；杀毒软件和磁盘对性能的影响。

## CMD-05 保存并重新打开一个 Command Block

- 状态：pending
- 用户或业务价值：用户创建或保存的 Command Block 在应用重启后仍可使用，不再依赖外部笔记。
- 参与者：Windows 用户、Command Block 作者。
- 触发与前置条件：Command Block Draft 已通过结构和模板校验。
- 预期结果：Command Block、分类、配置版本和必要参数状态写入 SQLite；重启后读取到等价对象；Built-in 与 User Origin 可区分。
- 验收场景：通过最小真实 Tauri 创建界面保存一个带 Folders 参数的用户命令，关闭并重新启动应用，重新打开后定义、模板和安全元数据保持一致。
- 来源需求或验收条件：创建、保存、分类、参数记忆；AC-07。
- 明确不包含：搜索体验、收藏、History、云同步、导入导出。
- 依赖：CMD-02。
- 风险或假设：Migration 必须原子失败；破坏性参数默认不自动记忆。

## CMD-06 从命令库找到并快速再次填写 Command Block

- 状态：pending
- 用户或业务价值：用户通过搜索、分类或收藏快速找到任务，并复用允许记忆的上次参数。
- 参与者：Windows 用户。
- 触发与前置条件：本地数据库中存在多个 Command Block。
- 预期结果：列表只加载 Summary；搜索、分类和收藏定位正确命令；选择后加载完整配置和符合策略的参数状态。
- 验收场景：通过最小真实 Tauri 命令库界面创建多条不同分类命令，使用关键字和收藏找到目标；普通参数恢复，破坏性路径参数不被意外恢复。
- 来源需求或验收条件：搜索、分类、收藏、减少重复输入；AC-07。
- 明确不包含：复杂全文检索、云端库、团队共享、Marketplace。
- 依赖：CMD-05。
- 风险或假设：启动和列表查询不能加载全部模板、日志或历史记录。

## CMD-07 查看一次 Execution 的稳定结果与完整历史

- 状态：pending
- 用户或业务价值：用户能追溯执行了什么、何时执行、持续多久、最终结果如何，并基于历史再次进入同一 Command Block。
- 参与者：Windows 用户。
- 触发与前置条件：至少完成过一次 Execution。
- 预期结果：History 分页显示 Execution Snapshot、Lifecycle、Outcome、Exit Code、耗时、目标级结果和日志元数据；日志按需读取且继续把 HTML、ANSI/OSC 控制序列和 URL 作为不可信惰性文本；数据库、日志或 UI 故障都不篡改外部命令 Outcome，并以独立记录状态呈现。用户可从 History 重新进入当前 Command Block，但历史 Snapshot 和 Hash 只用于追溯，不能授权新的 Run。
- 验收场景：通过最小真实 Tauri History 界面分别查看普通成功、特殊非零成功/警告、失败、部分失败、取消和目标状态未知的 Execution，并验证日志不解释 HTML、控制序列或链接；在 Hero Delete 中强制退出 Core，重启后显示持久化的可证明目标状态并将其余目标标为无法确认；再分别注入数据库完成记录失败、日志写失败、UI 慢消费或 Channel 断开，确认已经发生的外部 Outcome 保持不变、记录故障独立可见。最后从 History 重新进入当前 Command Block，按参数记忆策略填充并强制重新生成、确认当前 Preview；旧 revision、旧安全策略、Block 已删除或不可用时得到稳定提示且不启动进程。
- 来源需求或验收条件：Execution History、Log File、再次执行；AC-06、AC-07、AC-08。
- 明确不包含：远程日志、跨设备同步、无限日志保留、启动时扫描全部日志文件。
- 依赖：CMD-01、CMD-03、CMD-04、CMD-05。
- 风险或假设：Output 不能逐 Chunk 入库；日志必须有硬上限和清晰截断标记。

## CMD-08 通过完整桌面工作区管理和运行命令库

- 状态：pending
- 用户或业务价值：用户在一个连贯的桌面界面中完成导航、查找、编辑、参数填写、执行和结果查看。
- 参与者：Windows 用户、Command Block 作者。
- 触发与前置条件：CMD-01 至 CMD-07 均已通过各自最小真实 Tauri 垂直验收。
- 预期结果：Sidebar、Command List、Command Runner、History、Search、Favorite 和 Command Editor 形成完整闭环，各运行状态有明确按钮和反馈。
- 验收场景：从冷启动开始，找到快速删除命令、选择目录、Preview、执行、查看结果和 History；再创建一条用户命令并从搜索中重新打开。
- 来源需求或验收条件：完整产品 UI 与核心闭环；AC-01 至 AC-09 的 UI 汇总路径。
- 明确不包含：Interactive Terminal、Workflow、AI Builder、团队和 Marketplace。
- 依赖：CMD-01 至 CMD-07。
- 风险或假设：日志 Viewer 必须虚拟化；Execution Output 不进入全局 Zustand；不为了视觉完整提前扩大产品范围。

# 下一步

- 最近完成：`CMD-03`；`ATOMIC-CMD-03-001` 的五个原子、自动回归与安全真实宿主验收均已完成。
- 当前状态：用户已授权连续实施 `CMD-04`；`ATOMIC-CMD-04-001` 经两轮 L3 复核通过，当前从目标根安全与身份模型开始。
- 当前实施单元：`CMD-04` 永久删除多个超大目录；完成后停在 `CMD-05` 前等待用户检查和明确新授权。

# 计划变更记录

| 修订版本 | 变化 | 原因 |
|---|---|---|
| 1 | 首次建立 CmdBox MVP 产品拆分 | 将网页版 M0–M5 和最终产品边界转成可独立验收的产品单元 |
| 2 | 增加后端先行和前端原型确认门禁 | 用户授权先开发后端，并要求前端实现前先完成原型以避免返工 |
| 3 | 将 CMD-02 标记为完成并记录当前 Output 边界 | 九个原子、双 Runner、短等待 Cancel、响应式与安全验收已通过；项目进入用户检查关口 |
| 4 | 将 CMD-03 标记为完成并收窄目标级证据边界 | 五个原子、Outcome Policy、终态契约、结果卡与安全退出码/取消宿主矩阵通过；真实目标事实留给 CMD-04 |
| 5 | 启动 CMD-04 并增加破坏性实现门禁 | 用户明确授权继续；八个原子经两轮 L3 复核，默认 Registry 只在真实 Windows 安全矩阵通过后提升 |
