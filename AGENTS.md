# CmdBox Agent 工作说明

## 项目定位

CmdBox 是一个 Windows First 的桌面工具，把常用的一次性 CLI 命令封装成可参数化、可预览、安全执行并可重复使用的 Command Block。

当前 Hero Use Case 是：安全、快速地永久删除多个 Windows 超大文件夹。项目涉及不可逆文件操作，任何实现都必须优先维护 Preview 一致性、路径身份、进程树终止和可验证 Outcome。

## 每次接管必须先读

1. [项目工作台](docs/ai-project/项目工作台.md)：当前阶段、授权范围、风险和恢复入口。
2. [领域语言](CONTEXT.md)：Command Block、Preview、Execution、Lifecycle、Outcome 和 Command Workspace 的统一术语。
3. [活动产品拆分计划](docs/开发计划/产品拆分-CmdBox-MVP.md)：交付单元、依赖、状态和下一推荐单元。
4. 与当前单元直接相关的产品、架构和测试文档。
5. 全局及更近目录的 `AGENTS.md` 和用户专项规则仍然适用；本文件只补充 CmdBox 项目事实。

不得因为计划已经存在就自动进入代码实现。先以项目工作台的“当前授权”判断本次会话处于文档、设计、实现还是验证阶段。

## 当前 MVP 必要事实

- 平台：Windows First。
- 产品模式：一次性命令，`Input → Run → stdout/stderr → Exit`。
- 首个 Runner：Windows PowerShell；PowerShell/CMD 是第一阶段 Shell 契约。
- 技术栈：Tauri 2、React、TypeScript、Rust、SQLite。
- Rust Core 是进程、模板、安全、持久化和日志的信任边界。
- React 不得直接拼接 Shell 参数，也不得获得任意进程执行或任意 PID 终止能力。
- 当前不做：Interactive Terminal、PTY、持续 stdin、SSH 长连接、Workflow、云同步、团队、Marketplace、Plugin、Agent、定时任务。

## 开发环境入口

- 完整 Windows 桌面开发：`dev.cmd`；后台启动使用 `dev.cmd -Detached`，精确重启/停止使用 `restart.cmd` / `stop.cmd`。
- 纯前端 Docker 开发：`web-dev.cmd`；后台启动使用 `web-dev.cmd -Detached`，停止使用 `web-stop.cmd`。
- 日常小步验证：`pnpm check:fast`；提交前完整验证：`pnpm check`；完整 `pnpm tauri build` 只用于里程碑或发布。
- `src/` 修改走 Vite HMR，`src-tauri/src/` 修改走 Tauri Watch + Cargo Incremental；不要为普通源码修改手工重建 Bundle。
- 开发命令、依赖指纹、日志、故障处理和实测耗时统一见[开发环境与日常开发](docs/development/开发环境与日常开发.md)。
- `CMD-01` 固定无破坏任务闭环与 `CMD-02` 类型化预览和执行均已实现并实测。`CMD-02` 的 9/9 个原子已经完成：默认应用提供两个无破坏回显 Built-in，统一 Command Workspace 已接通六类 Typed Parameter、可信 `executionSpecHash`、一次性 Run 授权、per-run Channel、Output、Cancel 与唯一终态；91 项前端测试、PowerShell/CMD 真实回显、显式短等待 Cancel、三档响应式和无危险命令验收均通过。
- `CMD-02` 当前只表达 Execution Lifecycle、原始 Exit Code、耗时与有界实时 Output；Outcome、持久日志、虚拟化 Viewer、History、持久化和永久删除仍未实现。前端实时 Output 当前使用 512 KiB、2048 个非空 Chunk 的有界直接渲染。

## 不得破坏的全局约束

1. Preview 必须绑定完整 `ExecutionSpec`；Run 时重新读取、渲染、校验并比较 Hash。
2. Destructive Built-in 在 Preview 和 Run 两次执行 Safety Guard，并比较 `PathFingerprint`。
3. 路径安全检查只检查目标根对象，不递归扫描目录内容。
4. 参数必须 Typed Validation，并通过对应 Shell Serializer 进入脚本；不支持 Raw Parameter。
5. Cancel 针对整个 Execution 进程树；Windows 使用 Job Object，CmdBox Core 退出后受管任务不能继续失控运行。
6. 输出链路采用 Reader → Aggregator → Batch → Tauri Channel；UI 和日志不能给 Process Reader 施加阻塞背压。
7. Execution Lifecycle 与业务 Outcome 分离；特殊工具通过 `OutcomePolicy` 解释退出码。
8. 日志、数据库或 UI 保存失败不能改变已经发生的外部命令 Outcome。
9. Output 永远按不可信文本处理，不执行 HTML、控制序列或自动打开其中链接。
10. 永久删除功能必须在真实 Windows/Tauri 宿主中验证，不能只靠单元测试宣称完成。

## 文档职责与阅读优先级

同一职责只维护一份当前权威文档，不用 v0.1、v0.2 等并列文件保存讨论过程。过时内容必须先迁移仍然有效的事实，再删除被取代文件；工作台和阶段记录只维护状态、导航和关口，不复制产品或技术正文。

1. `产品需求.md` 是当前产品定义、MVP 范围、流程、规则和验收条件的唯一来源。
2. `技术设计.md` 是技术栈、模型、数据流、IPC 和持久化的唯一总体设计来源。
3. `安全与可靠性设计.md` 和 `性能设计.md` 是更具体的技术约束；发生冲突时，以更具体约束为准并同步修订总体技术设计。
4. `产品拆分-CmdBox-MVP.md` 负责交付单元、依赖和验收归属；不重新定义产品或架构。
5. `测试与验收.md` 负责验证场景、证据和发布门禁；尚未取得证据的能力不得描述为已实现。
6. 项目工作台记录当前授权。当前没有新的明确授权时，不得根据历史计划自动恢复代码或环境操作。

## 文档索引

| 类别 | 文档 | 用途 |
|---|---|---|
| 领域 | [领域语言](CONTEXT.md) | 项目专有术语及应避免的混用名称 |
| 视觉 | [Command Workspace 视觉原型](docs/design/Command-Workspace-视觉原型.png) | 用户确认的 `editorial-field-notes` 方案 1，前端视觉实现真值 |
| 视觉证据 | [Command Workspace 实现截图](docs/design/Command-Workspace-implementation-ready.png) | `1487 × 1058` 默认 Tauri Ready 状态实现证据 |
| 视觉验收 | [Command Workspace 视觉 QA](design-qa.md) | 源图、实现、交互、响应式与可访问性对照记录 |
| 项目状态 | [项目工作台](docs/ai-project/项目工作台.md) | 当前阶段、授权、风险、验收摘要和恢复入口 |
| 项目状态 | [项目阶段记录](docs/ai-project/项目阶段记录.md) | 阶段、关口和产物台账 |
| 产品 | [产品需求](docs/product/产品需求.md) | 当前定位、MVP、流程、业务规则和验收条件 |
| 技术 | [技术设计](docs/architecture/技术设计.md) | 技术栈、核心模型、数据流、IPC 和持久化 |
| 技术约束 | [性能设计](docs/architecture/性能设计.md) | Output、UI、日志、SQLite 和 Benchmark 约束 |
| 技术约束 | [安全与可靠性设计](docs/architecture/安全与可靠性设计.md) | 路径、进程树、Runner、Outcome 和故障隔离 |
| 开发 | [开发环境与日常开发](docs/development/开发环境与日常开发.md) | 环境检查、快速启动、Docker、Tauri、构建与故障处理 |
| 活动计划 | [CmdBox MVP 产品拆分](docs/开发计划/产品拆分-CmdBox-MVP.md) | 可独立验收的交付单元和依赖 |
| 已完成计划 | [开发环境与项目骨架原子计划](docs/开发计划/原子开发计划-开发环境与项目骨架.md) | 已验证的开发环境、Docker 和 Windows 主机入口准备记录 |
| 已完成计划 | [Command Workspace 前端视觉原型计划](docs/开发计划/原子开发计划-Command-Workspace前端视觉原型.md) | 已验证的无副作用 React 原型、交互与视觉 QA 记录 |
| 已完成计划 | [CMD-02 类型化预览与执行原子计划](docs/开发计划/原子开发计划-CMD-02类型化预览与执行.md) | 已验证的六类参数、可信 Preview、PowerShell/CMD 与统一宿主执行闭环记录 |
| 验收 | [测试与验收](docs/testing/测试与验收.md) | 实现及发布必须取得的验证证据 |

## 实现与文档同步

- 用户授权实现后，从活动计划中选择依赖已满足的推荐单元，不按前端、Rust、数据库横向拆成无法验收的半成品。
- `CMD-01` 与 `CMD-02` 已完成；当前没有进行中的代码原子，项目停在 `CMD-02` 用户检查关口。未经用户新的明确授权，不进入 `CMD-03`、永久删除或持久化实现。后续真实命令测试仍只允许回显、参数展示和短等待。
- 每个交付单元进入实现前，先规划最终用户流程和可观察结果。
- 实际实现、自动测试和真实宿主验证完成后，更新测试证据、项目工作台、阶段记录和计划单元状态。
- GitHub 公开仓库为 `https://github.com/C-Q-D/CmdBox`；每次完成有效改动并形成独立提交后，立即推送当前分支到对应远端跟踪分支。推送前必须完成适用验证并检查本次改动不含凭据或私有数据。
- 尚未实现或尚未实测的行为只能写为计划或草案，不得进入“功能使用流程”式事实文档。
- 后续设计变化直接更新对应唯一权威文档；确有长期价值且影响公共契约的决定才单独建立 ADR，避免以版本副本保存普通讨论过程。

<!-- codex-plan-index:start -->
## 当前计划索引

| 计划 ID | 类型 | 文档 |
|---|---|---|
| SCOPE-CMDBOX-001 | 产品拆分 | [CmdBox MVP 产品拆分](docs/开发计划/产品拆分-CmdBox-MVP.md) |

<!-- codex-plan-index:end -->
