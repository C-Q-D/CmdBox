# CMD-01 后端执行内核原子开发计划

## 计划元数据

- 计划 ID：ATOMIC-EXEC-BACKEND-001
- 类型：atomic-development
- 修订版本：1
- 状态：completed
- 父级 ID：不适用（支撑产品交付单元 `CMD-01`，但本计划完成不代表 `CMD-01` 完成）
- 创建基线：`30b0d4a`

## 总体计划

### 产品交付范围

- 当前要解决的问题：建立可在真实 Windows 上独立验证的 Rust 一次性 PowerShell 执行内核，使后续前端只需调用稳定的后端能力，而不用参与进程、安全和输出管理。
- 用户完成后能直接获得的结果：当前阶段可取得经过自动测试和真实 Windows 集成测试验证的后端执行能力；尚未接入界面，因此不得描述为用户可操作的 CmdBox 产品功能。
- 支持的产品单元：`CMD-01` 运行并控制一个固定的一次性 PowerShell 任务。
- 验收场景：后端测试调用固定 PowerShell 脚本，观察 Started、按协调器观察顺序排列的 stdout/stderr Output、自然 Finished、Exit Code 和耗时；取消时终止整个 Job；测试宿主被强制结束时不遗留受管子孙进程。
- 明确排除：`src/` 下全部前端代码、React、界面与样式、Tauri IPC command、模板参数、Preview Hash、SQLite、History、Hero Delete、CMD Runner、任意命令编辑。
- 前端门禁：本计划完成后必须停止；先与用户完成界面原型并获得确认，才可开发任何前端产品界面或前端调用链。

### 现状证据与架构接缝

- `src-tauri/src/lib.rs` 目前只装配并运行 Tauri Builder，没有业务 IPC 或系统权限。
- `src-tauri/Cargo.toml` 目前仅依赖 Tauri 2；执行内核所需依赖必须按当前真实 API 最小引入。
- `docs/architecture/技术设计.md` 已冻结 Windows PowerShell 参数、临时脚本、Job Object、输出协调器和 Execution 生命周期方向。
- `docs/architecture/安全与可靠性设计.md` 要求 `CREATE_SUSPENDED → AssignProcessToJobObject → ResumeThread`、`KILL_ON_JOB_CLOSE`、启动前脚本完整性复验和整树取消。
- `docs/architecture/性能设计.md` 要求 Reader 优先 Drain、跨流协调器、32 KiB 或 33 ms Batch、有界实时投递和慢消费者隔离。
- 当前基线命令 `cargo test --manifest-path src-tauri/Cargo.toml` 通过，共 0 个测试。

### 跨原子不变量

1. Windows PowerShell Runner 使用系统目录推导的绝对路径，不查找或信任可变 `PATH`。
2. 固定参数只能是 `-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File <script>`；不修改系统 Execution Policy。
3. 临时 `.ps1` 使用 UTF-8 BOM；调用 `CreateProcessW` 前紧邻启动点重新读取并比较 expected SHA-256，不匹配时不得 Resume。
4. 新进程必须先以挂起状态创建，成功加入设置了 `KILL_ON_JOB_CLOSE` 的独立 Job 后才恢复主线程；不启用 Breakaway。
5. Cancel 只表示请求被接受并进入 Cancelling；只有确认 Job 中受管进程树已经结束后才能发布 Cancelled。
6. stdout/stderr Reader 不能等待 UI 或未来 IPC 消费者；协调器在入口观察时分配全局 sequence，只承诺重建协调器观察顺序，不虚构两个 OS Pipe 的真实写入时序。
7. Started/Output/Finished 或 Cancelled 的接收端必须在 Resume 前绑定，避免短命进程造成先启动后订阅竞态。
8. Active Execution 全局锁只用于短时间插入、查询和移除，不得跨进程等待、输出读取或文件操作持有。
9. 每个人工维护的 Rust 源码和测试文件均补齐文件、类型、字段和函数层级中文注释。
10. 本计划不修改前端，不把底层或集成测试通过描述为真实 Tauri 用户路径完成。

### 原子顺序

1. `EXEC-BE-01`：确定性解析 Windows PowerShell Runner。
2. `EXEC-BE-02`：生成并复验临时 PowerShell Artifact。
3. `EXEC-BE-03`：创建并终止受 Job Object 管理的挂起进程。
4. `EXEC-BE-04`：非阻塞 Drain 并排序 stdout/stderr 输出。
5. `EXEC-BE-05`：组合固定脚本 Execution Session。

### 执行与 Git 策略

- 执行模式：连续执行；每个原子保持独立实现、验证、质疑和提交。
- 提交目标：GitHub 公开仓库 `https://github.com/C-Q-D/CmdBox`；每个原子提交完成后立即推送当前远端跟踪分支。
- 规划质疑：L2/L3 计划已完成一轮隔离审查；四项发现已全部吸收进本修订版本。
- 整体回归：`cargo fmt --manifest-path src-tauri/Cargo.toml --check`、`cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`、`cargo test --manifest-path src-tauri/Cargo.toml`、项目现有 `pnpm check`。

## EXEC-BE-01 确定性解析 Windows PowerShell Runner

- 状态：done
- 支持的验收场景：后端能确定当前系统的 Windows PowerShell 5.1 可执行文件和固定非交互参数。
- 唯一目标：返回不依赖 `PATH` 的 Windows PowerShell Runner 描述。
- 当前行为与目标行为：当前没有 Runner 模型；完成后 Rust 调用方可取得绝对 executable、稳定 runner 类型和构造固定脚本参数的方法。
- 前置条件与依赖：Windows 主机；无代码原子依赖。
- 代码定位依据：在真实消费者出现后新增 `src-tauri/src/process/windows/runner.rs` 及最小模块装配；`lib.rs` 只暴露后端模块，不增加 IPC。
- 允许修改：`src-tauri/Cargo.toml`、`Cargo.lock`、`src-tauri/src/lib.rs`、`src-tauri/src/process/**` 和直接测试。
- 明确不修改：进程启动、Job Object、临时脚本、输出、前端和 Tauri command。
- 实现步骤：使用 Windows 系统 API 取得系统 Windows 目录；拼接固定的 `System32/WindowsPowerShell/v1.0/powershell.exe`；验证为绝对且存在的文件；以结构化方法追加固定参数和脚本路径。
- 接口、数据与错误契约：Runner 类型稳定表示 `windowsPowershell`；解析失败返回结构化 Rust 错误并保留系统错误来源；不得回退到 `PATH` 或 `pwsh`。
- 边界与异常：系统目录 API 失败、拼接目标不存在、目标不是文件时失败；不处理 PowerShell 7。
- 测试要求：真实 Windows 上断言路径绝对、存在且文件名为 `powershell.exe`；断言固定参数顺序和脚本路径位于 `-File` 后；断言不读取测试注入的伪 `PATH`。
- 验证命令：针对 Runner 的窄测试；`cargo check --manifest-path src-tauri/Cargo.toml`。
- 预期结果：Runner 测试和 Rust 检查通过。
- 完成判定：后端可稳定取得唯一 Windows PowerShell 5.1 调用描述，未创建进程。
- 交付给下一原子的输出：临时 Artifact 和进程层可使用确定 executable 与固定参数。
- 停止或重新规划条件：当前 Windows API 无法可靠确定目标目录，或实际系统没有 Windows PowerShell 5.1。
- 风险等级：L2，Runner 路径属于执行安全契约。
- DDD 门禁：规划质疑已覆盖；提交前审查完整 diff 与 Runner 测试，必须为 PASS。
- 计划提交信息：`feat(core): [EXEC-BE-01] 确定 Windows PowerShell Runner`

### 执行记录

- 实际验证：Runner 窄测试 3 项通过；`cargo check --manifest-path src-tauri/Cargo.toml` 通过；`cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` 通过；提交前 DDD 复核为 `PASS`。

## EXEC-BE-02 生成并复验临时 PowerShell Artifact

- 状态：done
- 支持的验收场景：中文固定脚本以 Windows PowerShell 5.1 可确定读取的字节格式落盘，并能在启动前发现篡改。
- 唯一目标：创建持有 expected hash 且可在 spawn 前复验的临时 PowerShell Artifact。
- 当前行为与目标行为：当前没有临时脚本；完成后每次创建唯一目录和固定 `script.ps1`，完整写入、flush 并计算 SHA-256，调用方可在紧邻启动前再次校验。
- 前置条件与依赖：`EXEC-BE-01` 已提交。
- 代码定位依据：新增 `src-tauri/src/execution/artifact.rs` 和直接测试；不提前创建无消费者接口层。
- 允许修改：Rust manifest/lock、execution 模块、必要模块装配和直接测试。
- 明确不修改：Runner 解析、进程启动、Job Object、输出、前端。
- 实现步骤：在 CmdBox 专属临时根下创建随机 execution 目录；文件名固定；写 UTF-8 BOM 后写脚本文本；flush；读取字节计算 expected SHA-256；提供 `verify_before_spawn()` 再读并常量语义比较；显式清理并在 Drop 中尽力兜底。
- 接口、数据与错误契约：Artifact 持有脚本绝对路径、expected hash 和目录所有权；校验不匹配返回稳定的后端错误，禁止把路径或 hash 作为用户输入。
- 边界与异常：创建、写入、flush、读取、Hash 或清理失败分别保留 I/O 来源；Drop 失败不得 panic。
- 测试要求：验证 BOM 和中文正文；验证 expected hash 稳定；外部篡改后 `verify_before_spawn()` 失败；显式清理移除专属目录且不影响临时根其他内容。
- 验证命令：Artifact 窄测试；`cargo check --manifest-path src-tauri/Cargo.toml`。
- 预期结果：全部 Artifact 测试通过，无失败残留测试目录。
- 完成判定：后端可生成并在启动前复验一个自有临时脚本，但尚不创建进程。
- 交付给下一原子的输出：进程启动点获得可复验、可清理的脚本路径和 expected hash。
- 停止或重新规划条件：安全创建唯一目录无法由当前依赖可靠实现，或 Windows PowerShell 5.1 编码实测与 UTF-8 BOM 设计冲突。
- 风险等级：L2，Artifact 完整性属于执行安全接缝。
- DDD 门禁：规划质疑已覆盖；提交前审查完整 diff、篡改测试和清理边界，必须为 PASS。
- 计划提交信息：`feat(core): [EXEC-BE-02] 生成可复验 PowerShell Artifact`

### 执行记录

- 实际验证：Artifact 窄测试 3 项通过；`cargo check --manifest-path src-tauri/Cargo.toml` 通过；`cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` 通过；提交前 DDD 复核为 `PASS`。

## EXEC-BE-03 创建并终止受 Job Object 管理的挂起进程

- 状态：done
- 支持的验收场景：固定 PowerShell 进程及其子孙属于一个 Execution Job，自然结束可等待，取消或 Core 退出不会遗留受管树。
- 唯一目标：以无分配竞态的顺序创建、恢复、等待和终止一个 Windows Job 进程树。
- 当前行为与目标行为：当前没有进程能力；完成后进程以 `CREATE_SUSPENDED | CREATE_NO_WINDOW` 创建，加入设置 `KILL_ON_JOB_CLOSE` 的独立 Job 后才 `ResumeThread`。
- 前置条件与依赖：`EXEC-BE-01`、`EXEC-BE-02` 已提交；启动入口必须先调用 Artifact `verify_before_spawn()`。
- 代码定位依据：新增 `src-tauri/src/process/windows/job.rs`、`managed_process.rs` 及 Windows 集成测试 helper；调用 Win32 API，不用简单 spawn 后补分配。
- 允许修改：Rust manifest/lock、Windows process 模块、测试 helper 和直接集成测试。
- 明确不修改：stdout/stderr 文本读取和批处理、Execution Manager、IPC、前端。
- 实现步骤：复验 Artifact；创建设置 KILL_ON_JOB_CLOSE 的 Job；准备固定 executable、参数和安全 cwd；直接调用 `CreateProcessW` 挂起创建；Assign Job；Resume 主线程；关闭不再需要的线程句柄；提供事件驱动 wait 和 terminate Job；所有失败路径在 Resume 前终止未恢复进程并关闭已取得句柄。
- 接口、数据与错误契约：句柄由 RAII 类型唯一拥有；cancel 请求和“Job 已终止”是不同结果；不暴露任意 PID 终止能力；不设置 Breakaway。
- 边界与异常：Create Job、Set limits、Create Process、Assign、Resume、Wait、Terminate 分别返回含 Win32 来源的后端错误；失败不能泄漏可运行的未管理进程。
- 测试要求：真实 Windows 测试自然退出；运行会创建子进程的固定脚本后 Terminate Job，确认父子均消失；单独 helper Core 创建受管树后被测试进程强制结束，从外部确认子孙均消失；测试不得触碰用户文件。
- 验证命令：Windows managed process 窄测试（串行执行）；`cargo check --manifest-path src-tauri/Cargo.toml`。
- 预期结果：自然退出、整树取消和 Core 强退清理测试通过，测试结束无 helper 进程残留。
- 完成判定：Job 生命周期可独立验证；输出仍不进入 Rust 文本事件。
- 交付给下一原子的输出：输出层可以在同一原子边界内接入已创建的 stdout/stderr 管道句柄。
- 停止或重新规划条件：宿主已有 Job 限制导致 Assign 无法成立，或无法构造可重复的外部整树检查。
- 风险等级：L3，涉及并发、进程树终止和 Core 退出安全。
- DDD 门禁：规划质疑已覆盖；提交前审查完整 diff、句柄所有权、失败清理和真实 Windows 证据，必须为 PASS。
- 计划提交信息：`feat(core): [EXEC-BE-03] 使用 Job Object 管理进程树`

### 执行记录

- 实际验证：受管进程窄测试 3 项通过；启用 `process-test-helper` 的独立 Core 强退集成测试 1 项通过；`cargo check --manifest-path src-tauri/Cargo.toml` 通过；`cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --features process-test-helper -- -D warnings` 通过；提交前 DDD 复核为 `PASS`。

## EXEC-BE-04 非阻塞 Drain 并排序 stdout/stderr 输出

- 状态：done
- 支持的验收场景：固定脚本高频输出时，后端持续 Drain 两个 Pipe，并生成有序、有界的纯文本 Batch。
- 唯一目标：把受管进程的 stdout/stderr 转成不会被慢消费者反向阻塞的有序输出事件。
- 当前行为与目标行为：`EXEC-BE-03` 只管理进程；完成后两个 Reader 独立尽快读取，Aggregator 在入口观察时统一分配 sequence，并按 32 KiB 或 33 ms 形成 Batch。
- 前置条件与依赖：`EXEC-BE-03` 已提交。
- 代码定位依据：扩展 managed process 的继承管道接缝；新增 `src-tauri/src/execution/output.rs` 及直接/集成测试。
- 允许修改：Windows managed process 管道接缝、execution output 模块和直接测试。
- 明确不修改：Execution Session API、Tauri Channel、前端 Buffer、日志文件和 SQLite。
- 实现步骤：创建仅子进程继承的 stdout/stderr 写端；父进程关闭写端；两个 Reader 独立阻塞读取或异步桥接；每个片段进入单一 Aggregator 时立即取得 sequence；Batch 保存有序片段，仅合并相邻同 stream 片段；实时投递使用有界队列和非阻塞发送，慢消费时记录 dropped 状态但继续 Drain；使用状态型增量 decoder 保留跨 chunk 字符。
- 接口、数据与错误契约：输出事件包含 execution ID、sequence、stream、纯文本和 dropped 指示；sequence 只承诺协调器观察顺序；Output 读取失败与进程 Exit 分开表达。
- 边界与异常：空输出、无换行、中文跨 chunk、stdout/stderr 交错、消费者断开和队列满；不得 `from_utf8_lossy` 每块独立解码。
- 测试要求：验证相邻同流可合并但不会跨 stderr 重排；验证中文跨 chunk；高频输出加慢消费者时外部进程仍按时结束且队列容量有界；断开消费者后 Reader 仍 Drain 到 EOF。
- 验证命令：output 窄测试和 Windows 输出集成测试；`cargo check --manifest-path src-tauri/Cargo.toml`。
- 预期结果：顺序、增量解码、慢消费和断连测试通过。
- 完成判定：输出管道不会因当前实时消费者慢或消失而阻塞外部进程，且调用方可重建协调器观察顺序。
- 交付给下一原子的输出：Execution Session 可原子绑定事件接收端并发布生命周期和输出。
- 停止或重新规划条件：选定的 Pipe/异步桥接无法证明 Reader 与消费端隔离，或测试显示 Batch 会重排片段。
- 风险等级：L3，涉及并发、顺序和背压不变量。
- DDD 门禁：规划质疑已覆盖；提交前审查完整 diff、并发顺序和压力测试，必须为 PASS。
- 计划提交信息：`feat(core): [EXEC-BE-04] 有界聚合 PowerShell 输出`

### 执行记录

- 实际验证：输出模块测试 4 项通过（跨块 UTF-8、跨流顺序、真实 PowerShell 中文及 stdin EOF、慢消费者压力与有界队列）；受管进程窄测试 3 项通过；启用 `process-test-helper` 的独立 Core 强退集成测试 1 项通过；`cargo check --manifest-path src-tauri/Cargo.toml` 通过；`cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --features process-test-helper -- -D warnings` 通过；提交前 DDD 复核为 `PASS`。

## EXEC-BE-05 组合固定脚本 Execution Session

- 状态：done
- 支持的验收场景：后端调用方一次启动固定脚本并可靠接收完整生命周期，可按 Execution ID 取消或查询运行状态。
- 唯一目标：把 Runner、Artifact、Job Process 和 Output 组合成一个可独立集成测试的后端 Execution Session。
- 当前行为与目标行为：已有组件尚未形成调用入口；完成后 `start_fixed_powershell()` 原子返回 execution ID 与已在 Resume 前绑定的专属事件接收端，Manager 提供 cancel 和 active snapshot。
- 前置条件与依赖：`EXEC-BE-01` 至 `EXEC-BE-04` 均已提交。
- 代码定位依据：新增 `src-tauri/src/execution/session.rs`、`manager.rs` 及集成测试；`lib.rs` 只公开必要 Rust 后端 API，不注册 Tauri command。
- 允许修改：execution 组合模块、最小 app 装配、直接集成测试、项目状态与验收文档。
- 明确不修改：Tauri IPC、React、前端生成类型、模板、Preview、持久化、删除功能。
- 实现步骤：创建 execution ID、Artifact 和有界事件接收端；在 Resume 前完成接收端绑定并将 Active Execution 插入 Manager；复验 Artifact 后启动 Job；发布 Started/Output；wait 与 output drain 全部结束后发布 Finished；cancel 只把运行状态置为 Cancelling 并请求 Terminate Job，确认整树结束后才发布 Cancelled；最终移除 Active 并清理 Artifact。
- 接口、数据与错误契约：启动结果原子包含 `{execution_id, events}`；事件终态唯一；自然退出与 Cancel 竞态由已观测的进程终态决定；重复 cancel 返回稳定的 accepted/current-state 结果，不终止任意 PID。
- 边界与异常：极短脚本不能丢失 Started/Finished；空输出正常完成；非零 Exit Code 保留但本原子不解释 Outcome Policy；取消接近自然退出不产生双终态；启动失败清理 Active 和 Artifact。
- 测试要求：真实 Windows 集成测试覆盖极短自然结束、stdout/stderr、非零 Exit Code、运行中取消、重复取消、取消/自然退出竞态、查询 active、终态后移除和临时目录清理。
- 验证命令：Execution Session 窄测试；全部 Rust 测试；`cargo fmt --check`；`cargo clippy --all-targets -- -D warnings`；项目现有 `pnpm check`。
- 预期结果：后端调用链和全部回归通过；没有前端文件变化。
- 完成判定：固定 PowerShell 后端执行内核具备可复现证据；本计划改为 completed，活动索引移除，但产品 `CMD-01` 仍保持 pending，工作台进入“等待前端原型”门禁。
- 交付给下一原子的输出：原型确认后可单独规划 Tauri IPC 与前端最小用户路径。
- 停止或重新规划条件：生命周期无法保证单终态，或真实 Windows 测试发现取消/自然退出竞态仍可遗留进程。
- 风险等级：L3，涉及公共后端 API、并发状态与取消语义。
- DDD 门禁：规划质疑已覆盖；提交前审查完整 diff、生命周期测试和清理证据，必须为 PASS。
- 计划提交信息：`feat(core): [EXEC-BE-05] 组合固定脚本执行会话`

### 执行记录

- 实际验证：Execution Session 真实 Windows 测试 6 项通过，覆盖极短自然结束、stdout/stderr 中文、非零 Exit Code、Artifact 清理、根进程结束后的 Job 子孙清理、重复取消、取消/自然退出竞态、Active 查询和慢消费者有界压力；全部 Rust 测试 19 项通过；启用 `process-test-helper` 的独立 Core 强退进程树测试通过；`cargo fmt --check`、严格 `cargo clippy` 和项目 `pnpm check` 通过；完成端口 `ACTIVE_PROCESS_ZERO` 整树退出门禁经提交前 DDD 最终复核为 `PASS`。

# 计划变更记录

| 修订版本 | 变化 | 原因 |
|---|---|---|
| 1 | 首次建立后端执行内核计划，并吸收启动前 Hash 复验、预绑定订阅、跨流顺序和 Cancel 终态四项隔离审查结论 | 用户授权后端先行并要求前端开发前先确认原型 |
