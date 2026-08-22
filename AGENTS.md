# CmdBox Agent 工作说明

## 项目定位

CmdBox 是一个 Windows First 的桌面工具，把常用的一次性 CLI 命令封装成可参数化、可预览、安全执行并可重复使用的 Command Block。

当前 Hero Use Case 是：安全、快速地永久删除多个 Windows 超大文件夹。项目涉及不可逆文件操作，任何实现都必须优先维护 Preview 一致性、路径身份、进程树终止和可验证 Outcome。

## 每次接管必须先读

1. [项目工作台](docs/ai-project/项目工作台.md)：当前阶段、授权范围、风险和恢复入口。
2. [活动产品拆分计划](docs/开发计划/产品拆分-CmdBox-MVP.md)：交付单元、依赖、状态和下一推荐单元。
3. 与当前单元直接相关的产品、架构和测试文档。
4. 全局及更近目录的 `AGENTS.md` 和用户专项规则仍然适用；本文件只补充 CmdBox 项目事实。

不得因为计划已经存在就自动进入代码实现。先以项目工作台的“当前授权”判断本次会话处于文档、设计、实现还是验证阶段。

## 当前 MVP 必要事实

- 平台：Windows First。
- 产品模式：一次性命令，`Input → Run → stdout/stderr → Exit`。
- 首个 Runner：Windows PowerShell；PowerShell/CMD 是第一阶段 Shell 契约。
- 技术栈：Tauri 2、React、TypeScript、Rust、SQLite。
- Rust Core 是进程、模板、安全、持久化和日志的信任边界。
- React 不得直接拼接 Shell 参数，也不得获得任意进程执行或任意 PID 终止能力。
- 当前不做：Interactive Terminal、PTY、持续 stdin、SSH 长连接、Workflow、云同步、团队、Marketplace、Plugin、Agent、定时任务。

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

## 文档版本与阅读优先级

网页版已经讨论好的内容按版本分别保留，不要为了“整理”而覆盖或压缩原文。发生表面冲突时按以下顺序理解：

1. 用户原始需求是范围来源。
2. `MVP范围与核心场景.md` 是对较早 PRD 的后续收缩。
3. 技术设计 v0.2 补充和修订技术设计 v0.1。
4. 性能设计对两版技术设计增加性能约束。
5. 安全与可靠性设计 v0.3 对前述设计增加或修订安全、进程、Runner、Outcome 和故障隔离约束。
6. 项目工作台只记录当前事实；原始讨论文档继续作为设计依据，不在工作台复制全文。

三份长回复因当前 ChatGPT 会话读取接口单条 20000 字符上限只保存了可读取原文，文件头已标注。不得把自行补写的内容冒充缺失原文；实现前应根据当前依赖版本、Windows API 和真实实验复核具体接口细节。

## 文档索引

| 类别 | 文档 | 用途 |
|---|---|---|
| 项目状态 | [项目工作台](docs/ai-project/项目工作台.md) | 当前阶段、授权、风险、验收摘要和恢复入口 |
| 项目状态 | [项目阶段记录](docs/ai-project/项目阶段记录.md) | 阶段、关口和产物台账 |
| 产品来源 | [原始需求与命令案例](docs/product/原始需求与命令案例.md) | 用户原始需求和 Windows 命令案例 |
| 产品定义 | [产品定义](docs/product/产品定义.md) | 当前有效定位、MVP、非目标和不变量 |
| 产品来源 | [产品需求文档 PRD v0.1](docs/product/产品需求文档-PRD-v0.1.md) | 网页版早期完整 PRD 原文 |
| 产品来源 | [MVP 范围与核心场景](docs/product/MVP范围与核心场景.md) | 一次性命令和 Hero Delete 的后续确认 |
| 技术来源 | [技术选型](docs/architecture/技术选型.md) | Tauri/React/Rust/SQLite 与 Windows First 选型 |
| 技术来源 | [技术设计 v0.1](docs/architecture/技术设计-v0.1.md) | Schema、模板、Execution、数据库和模块初版设计 |
| 技术来源 | [技术设计 v0.2](docs/architecture/技术设计-v0.2.md) | Contract、Channel、IPC、Repository 等细化设计 |
| 技术约束 | [性能设计](docs/architecture/性能设计.md) | Output、UI、日志、SQLite 和 Benchmark 约束 |
| 技术约束 | [安全与可靠性设计 v0.3](docs/architecture/安全与可靠性设计-v0.3.md) | 路径、进程树、Runner、Outcome 和故障隔离 |
| 规划来源 | [开发路线](docs/planning/开发路线.md) | 网页版最终 M0–M5 开发顺序 |
| 活动计划 | [CmdBox MVP 产品拆分](docs/开发计划/产品拆分-CmdBox-MVP.md) | 可独立验收的交付单元和依赖 |
| 验收 | [测试与验收](docs/testing/测试与验收.md) | 实现及发布必须取得的验证证据 |

## 实现与文档同步

- 用户授权实现后，从活动计划中选择依赖已满足的推荐单元，不按前端、Rust、数据库横向拆成无法验收的半成品。
- 每个交付单元进入实现前，先规划最终用户流程和可观察结果。
- 实际实现、自动测试和真实宿主验证完成后，更新测试证据、项目工作台、阶段记录和计划单元状态。
- 尚未实现或尚未实测的行为只能写为计划或草案，不得进入“功能使用流程”式事实文档。
- 原讨论文档作为来源保留；后续设计变化应建立明确的新修订或决策记录，不静默改写历史原文。

<!-- codex-plan-index:start -->
## 当前计划索引

| 计划 ID | 类型 | 文档 |
|---|---|---|
| SCOPE-CMDBOX-001 | 产品拆分 | [CmdBox MVP 产品拆分](docs/开发计划/产品拆分-CmdBox-MVP.md) |
| ATOMIC-ENV-001 | 原子开发 | [开发环境与项目骨架原子计划](docs/开发计划/原子开发计划-开发环境与项目骨架.md) |

<!-- codex-plan-index:end -->
