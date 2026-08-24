# CMD-04 安全快速永久删除原子开发计划

## 计划元数据

- 计划 ID：ATOMIC-CMD-04-001
- 类型：atomic-development
- 修订版本：1
- 状态：in_progress
- 父级 ID：CMD-04
- 创建基线：862b927

## 总体计划

- 用户结果：用户在真实 Windows/Tauri 中选择一个或多个专用目录，经可信 Preview 后永久删除，并看到 Rust Core 提供的逐目标结果；系统直接拦截关键根、顶层 Reparse Point 和已经变化的目标。
- 最终用户流程：打开“快速永久删除多个文件夹” → 选择目录 → Rust 规范化、去重、祖先折叠并检查目标根 → 查看“不经过回收站”的 Preview、安全结论和目标摘要 → 必要时输入 `DELETE` 强化确认 → 永久删除 → 查看 Lifecycle、Outcome、Exit Code、耗时和逐目标结果；运行中可取消但不会回滚已经完成的删除。
- 当前行为：两个正式无破坏 Echo Built-in 已具备类型化参数、Preview/Run Hash、受管进程树、Output、Cancel 和 Outcome；没有破坏性 Definition、Path Fingerprint、Single Flight、真实删除 Executor 或目标结果 wire contract。
- 实现边界：只交付 Built-in Windows PowerShell 多目录永久删除、根级安全检查、逐目标事实、取消和 Hero Delete 专项基准；不交付回收站、递归安全扫描、删除链接本身、History、SQLite、重启恢复、复制或移动。
- 路径身份不变量：目标必须是绝对、存在的目录；顶层 Reparse Point、卷根、UNC Share 根和关键系统/应用根直接拦截；Preview 与 Run 使用 Final Path、卷序列号和 128-bit File ID 建立 `PathFingerprint`，并进入升级后的 Canonical Execution Spec。
- 安全检查边界：Safety Guard 只读取目标根对象，不递归扫描内容；输入路径和 Final Path 都参与保护路径判断。Desktop、Documents、Downloads 和 OneDrive 精确根只进入 `highRisk` 强化确认，普通子目录不触发。
- 竞态边界：Run 全量重建后，固定 Executor 对每个目标 `BEGIN`，Rust 紧邻副作用再次核验身份并决定授权；授权到 PowerShell 按路径删除之间仍有窄 TOCTOU 残余风险。本计划不把路径删除宣称为针对同用户主动对抗替换的原子操作；若内部链接或竞态门禁证明当前策略可能越界，立即停止并更换 Executor。
- 执行事实不变量：目标状态只来自认证的私有 collector 协议和 Run 时已验证的 index→path 映射，永不解析 stdout/stderr，也不接受 side channel 提供路径。三种终态都发布完整、稳定顺序的 `targetResults`；普通命令固定为空数组。
- Collector 状态机：每个 index 必须先同步写入 `BEGIN`，Rust 复验并按 execution token、index 和 generation 返回 approval/deny；脚本有界等待，超时或 deny 不执行删除。`SUCCESS` 只在 `Remove-Item -LiteralPath -Recurse -Force -ErrorAction Stop` 返回且目标根确认不存在后写入；`FAILURE` 使用稳定原因类别。无 `BEGIN` 只有在协议前缀完整且顺序可证时才是 `notStarted`；`BEGIN` 无终态或协议损坏均保守为 `unableToConfirm`。
- Transport 与 Hash：新版 Spec Hash 覆盖 collector protocol version、逻辑占位符和完整目标 index 映射；每次 Execution 的随机 transport 路径与认证 token 是唯一允许的 launch-local 实例值，不冒充 Canonical Environment，也不进入业务 Hash。Transport、结果 sink、工作目录、Artifact 和全部目标必须动态判定互不相交。
- Collector 生命周期：collector lease 独立于 `ProcessLaunch` / `MaterializedScript`；Job 与根进程结束后，在 lease 仍存活时读取并验证结果，再生成终态，最后统一清理。读取失败不得乐观推断。
- Handshake 故障边界：watcher 和 endpoint 必须在 Resume 前 ready；启动、运行、解析或写 approval 失败必须 fail-closed 并终止整个 Job。Cancel 同时停止 watcher；超时、失败和取消都必须收敛到唯一终态，释放进程、reservation、Artifact 与 transport。
- Outcome 映射：`confirmedDeleted → Success`，`failed → Failure`，`notStarted | unableToConfirm → Unknown`；完整可信的 Finished facts 才可由 Target Results Policy 生成 success/partialFailure/failure。Cancelled 和内部 Failed 的 Outcome 固定为 `none`，但仍携带当前可证明的逐目标事实。
- Single Flight：在 `verify_run` 成功后、创建 Artifact、transport、PreparedProcess 或 Started 事件之前，按 destructive `executionSpecHash` 在 Manager 同一锁中原子 reserve；所有启动失败自动释放，成功后由 Supervisor 在唯一终态入队后释放。
- Registry 门禁：永久删除 Built-in 在前六个原子只存在于 `delete-validation` feature；完整真实 Windows/Tauri 安全矩阵通过后，才在宿主原子提升到默认 Registry，并用默认构建重跑受控成功删除和 Cancel 冒烟测试。
- 测试数据边界：任何真实删除只作用于测试当次创建、绝对路径经过核对的 UUID 隔离根；外部 sentinel 只用于证明内部 Junction/Symlink 不越界；禁止把项目目录、用户目录、系统目录或既有业务数据作为删除目标。
- 整体回归：每个原子执行窄测试；最后执行 `pnpm check`、feature/default Rust 回归、Windows 集成、真实 Tauri、安全残留检查和 Terminal/CmdBox 同数据集基准。
- Git：每个原子独立验证、提交，并立即推送当前 `master` 到 `origin/master`。
- 计划质疑：首轮 L3 复核要求补齐 collector 信任、Hash、逐目标授权、内部链接、reserve 时序、确认契约、Registry 与性能原子；第二轮补齐 Artifact 生命周期、必填目标结果、默认提升门禁和 handshake fail-closed 后结论为 `PASS`。

## 原子依赖

```text
CMD04-SAFETY-01 → CMD04-SPEC-01 → CMD04-EXECUTOR-01 → CMD04-SESSION-01
→ CMD04-CONTRACT-01 → CMD04-UI-01 → CMD04-WINDOWS-HOST-01 → CMD04-BENCH-DOC-01
```

## CMD04-SAFETY-01 建立 Windows 目标根安全与身份模型

- 状态：done
- 支持的验收场景：普通目录可 Preview；不存在、文件、危险 Namespace、卷根、UNC Share 根、系统关键目录和顶层 Reparse Point 被拒绝；高风险用户目录被准确分级；重复项与子目录被折叠。
- 唯一目标：建立只读取目标根、可在 Preview/Run/逐目标授权复用的 `DeletePathsSafetyGuard` 深模块。
- 当前行为与目标行为：Folders 参数只有词法规范化和基础目录元数据；完成后 Safety Guard 返回规范化目标集、Final Path、稳定 128-bit 身份、风险级别、折叠事实和稳定错误码。
- 前置条件与依赖：CMD-01 至 CMD-03 已完成；无本计划内依赖。
- 代码定位依据：`src-tauri/src/execution/parameter.rs`、新增 `execution/safety/` Windows 模块、`windows-sys` FileSystem/Shell/SystemInformation API 和直接测试。
- 允许修改：纯路径策略、Windows Handle RAII、Known Folder/应用数据注入接缝、稳定安全错误、测试辅助目录。
- 明确不修改：Command Registry、模板、Spec、Session、IPC、React 和真实删除。
- 实现步骤：定义 `DeletePathPolicy`；拒绝危险 Namespace；以 Windows 不区分大小写语义去重和祖先折叠；读取顶层属性；打开目录 Handle；获取 Final Path、卷序列号与 128-bit File ID；对输入/Final 双重判断 critical/highRisk；输出不含递归统计的安全报告。
- 接口、数据与错误契约：错误不回显未经验证的任意路径；Fingerprint 使用确定 UTF-16 路径语义和固定字节身份；Known Folder 来自 Windows/Tauri 路径 API 而非硬编码盘符。
- 边界与异常：UNC、Extended Path、中文、空格、大小写、末尾分隔符、`.`/`..`、不可读对象、目标在检查中消失；任何不确定 critical 判定均 fail-closed。
- 测试要求：纯比较矩阵、真实 UUID 目录身份稳定/重建变化、顶层 Junction/Symlink、不同大小写重复、父子折叠、Known Roots 分类；证明检查次数只随目标数增长，不枚举子树。
- 验证命令：Safety 窄测、完整 Rust 单测、format、strict Clippy；不运行删除命令。
- 预期结果：同一目录对象得到稳定 Fingerprint，替换对象身份变化，危险目标不能进入后续 Spec。
- 完成判定：根级矩阵通过，新增模块没有执行、删除或进程副作用。
- 交付给下一原子的输出：类型化 `SafetyReport`、`PathFingerprint`、折叠后的目标映射和稳定错误。
- 停止或重新规划条件：必须递归扫描才能判断根安全，或 Windows API 无法提供稳定对象身份。
- 风险等级：L3
- DDD 门禁：提交前限定范围复核必须 `PASS`。
- 计划提交信息：`feat(safety): [CMD04-SAFETY-01] 建立目标根安全身份`

### 执行记录

- 实际交付：新增只检查目标根的 Windows Safety 深模块；使用不跟随顶层 Reparse Point 的目录 Handle 读取属性、DOS Final Path、卷序列号和 128-bit File ID；通过 Windows ordinal ignore-case 完成去重与祖先折叠；由 System、Known Folder 和 Tauri App Data 接缝建立 critical tree、Profile exact 与用户 high-risk exact 分级。危险 Device Namespace 同时按规范化分隔符文本和 Windows Prefix kind 拒绝，Final Path Extended 前缀全程按原始 UTF-16 处理。
- 实际验证：Safety 9 项全部通过，覆盖目录重建身份变化、DOS/UNC/Extended UNC 根、反斜杠/正斜杠 Device Namespace、顶层 Junction、内部 Junction 与深层内容不递归扫描、保护根分级、重复和父子折叠；完整 Rust 为 100 passed、1 ignored，前端 97 passed，strict Clippy、format、diff check 通过，`safety-*` 临时目录残留为 0。
- 安全边界：测试只创建和清理 `%TEMP%\CmdBox\safety-<label>-<UUID>`；一次无权限 Symlink 夹具失败遗留的空测试根在验证 UUID 与 Temp 边界后清理。当前模块未接入 Planner、默认 Registry 或任何 UI 删除入口，不产生删除业务副作用。
- 复核结果：首轮 L3 发现替代分隔符 Namespace、lossy UTF-16、panic 清理和测试证据不足；修订后第 2 轮限定复核为 `PASS`。
- 计划偏差：为兼容当前未提权 Windows 测试环境，顶层 Reparse Point 夹具使用不要求 Symlink 特权的本地 Junction；被测 Windows 属性和产品安全语义不变。

## CMD04-SPEC-01 绑定破坏性 Definition、Safety 与新版 Spec

- 状态：done
- 支持的验收场景：永久删除命令经 feature Registry 读取；Preview 展示完整目标摘要、安全结论和明确动作；Run 二次检查、目标变化与强化确认均由后端裁决。
- 唯一目标：把 Delete Safety、确认要求和目标身份纳入唯一 Preview/Run Canonical Spec。
- 当前行为与目标行为：正式 Definition 只有 normal Echo，Spec Schema 不含 Fingerprint；完成后 feature-only Delete Definition 使用 Target Results Policy，Hash 覆盖完整破坏性事实。
- 前置条件与依赖：`CMD04-SAFETY-01`。
- 代码定位依据：`command.rs`、`planner.rs`、`spec.rs`、Serializer/Template 和 `delete-validation` feature。
- 允许修改：内部 SafetyPolicy、Delete Definition、固定 PowerShell 模板、Preview DTO 内部内容、Spec Schema 与 Hash 测试、Run confirmation request。
- 明确不修改：默认 Registry、进程启动、Target collector、公开终态、React。
- 实现步骤：注册 feature-only destructive Built-in；限制 `folders` 数量且默认不记忆；让 Preview/Run 共用 Guard；Hash fingerprints、index mapping、Safety decision、confirmation requirement/version 与 collector protocol placeholder；blocked 不产出授权值；highRisk 在 fresh Guard 后验证独立 confirmation response。
- 接口、数据与错误契约：Preview 动作为“永久删除”；稳定区分 `SAFETY_BLOCKED`、`TARGET_CHANGED`、`CONFIRMATION_REQUIRED`；用户输入 `DELETE` 不作为普通 Parameter 或 Preview 前 Hash 值。
- 边界与异常：折叠导致目标列表变化、Policy/协议/确认版本变化、Preview 后删除重建或替换成链接、普通与 highRisk 混合目标。
- 测试要求：Schema 每个新增组件改变 Hash；同输入稳定；Run fresh Guard；blocked/changed/confirmation；正常 Preview 文本使用 `-LiteralPath` 且不泄露 launch-local transport。
- 验证命令：Command/Planner/Spec 窄测、feature/default Registry、Contract 前置回归、format、strict Clippy；不启动删除。
- 预期结果：只有当前完整安全事实和必要确认都匹配时才能获得私有 Verified Delete 值。
- 完成判定：默认仍只有两个正式 Built-in，feature Delete 可 Preview/复验但尚不启动。
- 交付给下一原子的输出：Hash 已绑定的目标映射、期望 Fingerprint、固定脚本和 collector 逻辑契约。
- 停止或重新规划条件：随机 transport 值必须进入 Hash，或确认只能由前端放行。
- 风险等级：L3
- DDD 门禁：提交前限定范围复核必须 `PASS`。
- 计划提交信息：`feat(core): [CMD04-SPEC-01] 绑定永久删除执行规范`

### 执行记录

- 实际交付：新增仅在 `delete-validation` 注册的永久删除 Built-in、`DeletePaths` Safety Policy 和 Spec Schema 3；Preview/Run 每次重新建立系统保护根、执行根级 Guard，并把折叠后的有序目标路径、Final Path、卷序列号、128-bit File ID、安全判定、确认要求版本、collector 协议版本和 Outcome Policy 版本纳入 Canonical Hash。Run 强制提交独立目标身份凭据，high-risk 仅接受当前 v1 的精确 `DELETE` 响应。
- 启动门禁：Hash 已验证的 delete 值保留目标 Fingerprint 与 collector 协议版本，但在可信 Executor 完成前 `launchReady=false`；唯一生产 Session 启动边界返回 `EXECUTOR_UNAVAILABLE`，不会创建 Artifact 或进程。默认 Registry 仍只有两个正式 Echo Built-in。
- 安全收敛：critical tree 与候选目标双向重叠均阻断；critical exact 根及其祖先阻断、其普通子目录仍允许。Risk/Safety 类型错配、零版本和未实现的 Safety/confirmation/collector 版本全部在进入 Spec 前 fail-closed。
- 实际验证：默认 Rust `101 passed / 1 ignored`，`delete-validation` 为 `106 passed / 1 ignored`，全 feature 为 `107 passed / 1 ignored`，Windows 进程树集成 `1 passed`；前端 `97 passed`、TypeScript、Vite build、契约生成/漂移、format、strict Clippy 和 diff check 通过；`spec-*`、`safety-*` 临时目录残留为 0。
- 测试边界：本原子没有执行永久删除模板；文件系统测试只创建、重建和清理 `%TEMP%\CmdBox\spec-<UUID>` / `safety-<label>-<UUID>` 隔离目录，并只读检查卷根阻断。
- 复核结果：首轮 L3 发现 critical 根祖先未阻断、Verified 值丢失 collector 版本和未知确认版本未 fail-closed；修订后第 2 轮限定复核为 `PASS`。
- 计划偏差：Run/Preview 新增公开字段要求同步生成 TypeScript contract，因此在本原子前置完成对应生成文件；目标结果和完整公开终态契约仍留在 `CMD04-CONTRACT-01`。

## CMD04-EXECUTOR-01 建立可信逐目标删除协议

- 状态：in_progress
- 支持的验收场景：隔离目录逐个删除；每个目标在副作用前重新核验；成功、失败、未开始和无法确认来自严格类型化事实；内部链接不能改动外部 sentinel。
- 唯一目标：建立 fail-closed、可取消且不解析 Output 的 Delete Executor/collector 深模块。
- 当前行为与目标行为：Session 只管理 stdout/stderr 与 Exit Code；完成后 Delete Verified 值可物化独立 collector lease、认证 handshake 和类型化目标事实。
- 前置条件与依赖：`CMD04-SPEC-01`。
- 代码定位依据：新增 `execution/delete_executor.rs` 与协议模块、`artifact.rs`、`planner.rs`、PowerShell Serializer 和 Windows 隔离测试。
- 允许修改：固定脚本、launch-local transport、watcher、approval、严格 parser、collector lease 和测试 helper。
- 明确不修改：Manager Single Flight、公开 IPC、React、默认 Registry、业务目录。
- 实现步骤：Resume 前建立 ready watcher；每目标同步 BEGIN；认证 approval/deny；Rust 紧邻复验；PowerShell 有界等待并仅在 approval 后删除；同步 Success/Failure；严格解析；结果读取先于 lease 清理；所有路径做不相交检查。
- 接口、数据与错误契约：transport 只用 token/index/generation；结果路径由 verified map 恢复；协议异常使未确定项全部 `unableToConfirm`；失败消息有界且不包含任意 PowerShell 对象序列化。
- 边界与异常：watcher-not-ready、解析/写 approval 失败、超时、Cancel during handshake、脚本被 Job 终止、结果文件部分写入、清理失败、目标在 approval 前漂移。
- 测试要求：上述故障均无删除、无挂起、无 transport 残留；SUCCESS 必须根不存在；失败后继续下一目标；内部 Junction/Symlink 指向隔离根外 sentinel 的真实 Windows 门禁。
- 验证命令：Executor 窄测、feature Rust 回归、Windows 隔离集成、format、strict Clippy。
- 预期结果：collector 可提供完整、保守、按目标稳定排序的事实，stdout/stderr 内容不影响它。
- 完成判定：只删除测试创建的 UUID 目标；越界 sentinel 不变；故障全部 fail-closed。
- 交付给下一原子的输出：可交 Supervisor 管理的 delete launch、watcher 与 collector lease。
- 停止或重新规划条件：内部链接测试越界、approval 前核验无法执行，或任何协议故障可能继续删除。
- 风险等级：L3
- DDD 门禁：提交前限定范围复核必须 `PASS`。
- 计划提交信息：`feat(executor): [CMD04-EXECUTOR-01] 建立可信删除目标协议`

## CMD04-SESSION-01 集成 Single Flight、取消与目标终态

- 状态：todo
- 支持的验收场景：并发双击只启动一个删除 Execution；自然结束、取消、内部失败和握手失败均唯一收敛并释放资源。
- 唯一目标：把 destructive reservation、collector 生命周期和 Target Results Policy 集成到现有 Session Supervisor。
- 当前行为与目标行为：Manager 仅按 Execution ID 在 PreparedProcess 后登记；完成后 destructive Hash 在任何启动副作用前 reserve，终态携带完整目标事实。
- 前置条件与依赖：`CMD04-EXECUTOR-01`。
- 代码定位依据：`manager.rs`、`session.rs`、`planner.rs`、`managed_process.rs` 和线程/竞态测试。
- 允许修改：Manager 复合索引、RAII reservation、启动时序、Supervisor、内部终态和测试。
- 明确不修改：公开 wire、React、默认 Registry、普通命令的用户行为。
- 实现步骤：先分配 Execution ID 并 reserve hash；再物化 Artifact/transport、prepare、ready watcher、登记 Active、Resume；Supervisor 等 root/Job，读取 collector，停止/join watcher与 Output，发布唯一终态，再 release/cleanup；所有 error 分支统一收口。
- 接口、数据与错误契约：重复返回 `DUPLICATE_EXECUTION`；普通命令不进入 destructive hash index；三终态内部均有 `target_results`，normal 为空。
- 边界与异常：Artifact/transport/prepare/resume/thread spawn 失败、自然结束与 Cancel 竞态、handshake Cancel、Channel 断开、Core 强退、poisoned lock。
- 测试要求：任一步失败无 reservation；同 Hash 并发仅一个外部进程；不同 Hash 可并行；A/B/C Cancel 事实；读取先于 collector 清理；唯一终态和 Active/Job/线程清理。
- 验证命令：Manager/Session 窄测、feature/default Rust、Windows Core 集成、format、strict Clippy。
- 预期结果：不可逆任务不会因 UI 双击重复启动，任何失败或取消都不会永久占用 Single Flight。
- 完成判定：所有启动和终态路径都可证明资源释放，既有 normal 执行/取消不回归。
- 交付给下一原子的输出：内部完整的 Lifecycle + Outcome + Target Results 终态。
- 停止或重新规划条件：reserve 只能发生在进程创建后，或 collector 读取必须晚于 Artifact 清理。
- 风险等级：L3
- DDD 门禁：提交前限定范围复核必须 `PASS`。
- 计划提交信息：`feat(core): [CMD04-SESSION-01] 集成删除任务终态与单飞`

## CMD04-CONTRACT-01 发布安全与逐目标契约

- 状态：todo
- 支持的验收场景：前端只消费 Rust 生成的 Safety、稳定错误和逐目标结果，不增加任意脚本或 PID 能力。
- 唯一目标：以 ts-rs 单一真值发布 CMD-04 所需的最小 Typed IPC Contract。
- 当前行为与目标行为：终态只有 Outcome；完成后三种终态必填 `targetResults`，normal 为空，delete 返回完整稳定列表。
- 前置条件与依赖：`CMD04-SESSION-01`。
- 代码定位依据：`ipc/execution.rs`、生成契约、Gateway 和前后端 Fixture。
- 允许修改：Preview 安全详情、Run confirmation response、Target Result DTO、终态映射、公开错误和生成文件。
- 明确不修改：五个业务 IPC 集合、脚本/Executable/PID 暴露、React 展示逻辑、默认 Registry。
- 实现步骤：定义 `confirmedDeleted | failed | notStarted | unableToConfirm`；映射 verified path 与稳定 reason；三终态必填数组；生成 TS；收敛安全/变化/确认/重复错误。
- 接口、数据与错误契约：请求仍只含 ID、revision、结构化参数、Hash 和必要 confirmation；路径结果来自后端 verified map；不跨 IPC 发布 Fingerprint、token、transport 或 Policy。
- 边界与异常：normal 空数组、协议失败完整 unknown 列表、Cancelled/Failed outcome none、未知字段拒绝、数组与文本有界。
- 测试要求：serde 白名单、ts-rs drift、Gateway 五命令、所有终态/错误映射、TypeScript typecheck 和现有 Fixture 回归。
- 验证命令：Contract generate/check、IPC 窄测、前端 test/typecheck、Rust 回归。
- 预期结果：React 无需解析 stdout 或猜测目标状态。
- 完成判定：生成契约无漂移，公开面未扩大为任意执行。
- 交付给下一原子的输出：完整 destructive Preview/Run/Result 类型。
- 停止或重新规划条件：必须发布 transport、脚本、任意路径探测或 PID 才能工作。
- 风险等级：L3
- DDD 门禁：提交前限定范围复核必须 `PASS`。
- 计划提交信息：`feat(ipc): [CMD04-CONTRACT-01] 发布永久删除目标契约`

## CMD04-UI-01 接入永久删除工作区

- 状态：todo
- 支持的验收场景：用户能选择目录、查看安全 Preview、完成强化确认、永久删除或取消，并看到逐目标状态。
- 唯一目标：在已确认的 Command Workspace 视觉语法内完成 destructive 用户流程，仍保持 feature-only Registry。
- 当前行为与目标行为：Workspace 已有通用参数/Preview/执行结果区域但只运行 normal 命令；完成后按后端事实显隐破坏性区域和目标列表。
- 前置条件与依赖：`CMD04-CONTRACT-01`。
- 代码定位依据：`CommandWorkspace.tsx`、`ParameterForm.tsx`、Gateway、现有 CSS 与 App 测试。
- 允许修改：destructive 内容分支、确认输入、按钮/说明、Target Results 列表和最小样式。
- 明确不修改：整体三栏结构、视觉重设计、前端 Safety 推断、输出解析、默认 Registry。
- 实现步骤：显示“不经过回收站”、目标数量/折叠数/警告；blocked 禁止 Run；highRisk 输入 `DELETE`；使用 `actionLabel`；运行/取消文案强调不回滚；终态按后端数组展示。
- 接口、数据与错误契约：前端只提交确认 response；参数变化、安全响应变化和 generation 变化立即使旧确认/Preview 失效；不从 Exit Code 或文本推断状态。
- 边界与异常：重复点击、Preview 迟到、确认变化、Run 拒绝、取消竞态、超长路径列表截断、键盘与窄窗口。
- 测试要求：passed/warning/blocked、DELETE、折叠、四种目标状态、partialFailure、Cancel 不回滚、旧事件隔离、不可信文本、响应式和可访问性。
- 验证命令：Workspace 窄测、完整前端测试、typecheck、build、contract drift。
- 预期结果：feature 构建已有完整可操作 UI，但默认产品仍未公开未过宿主门禁的删除命令。
- 完成判定：既有视觉语法和 normal 命令无回归，前端没有安全/Outcome 业务推断。
- 交付给下一原子的输出：可供真实 Windows/Tauri 安全矩阵验收的完整 feature UI。
- 停止或重新规划条件：必须重构整体界面或增加任意执行能力。
- 风险等级：L3
- DDD 门禁：提交前限定范围复核必须 `PASS`。
- 计划提交信息：`feat(ui): [CMD04-UI-01] 接入安全永久删除工作区`

## CMD04-WINDOWS-HOST-01 通过真实 Windows 安全门禁并提升默认 Registry

- 状态：todo
- 支持的验收场景：真实 Tauri 下成功、部分失败、安全拦截、变化、重复、取消、内部链接和 Core 强退均满足 CMD-04。
- 唯一目标：以真实 Windows/Tauri feature 矩阵证明策略安全后，才把永久删除提升为默认 Built-in。
- 当前行为与目标行为：feature UI 尚无完整宿主证据；完成后矩阵全通过、默认 Registry 含 Hero Delete，并重跑默认受控 smoke。
- 前置条件与依赖：`CMD04-UI-01`。
- 代码定位依据：feature Registry、Windows 集成测试/隔离 helper、真实 Tauri、Command/Session/IPC 和默认 Registry。
- 允许修改：验收 helper、必要故障注入、门禁通过后的最小 Registry flip 和直接测试。
- 明确不修改：非隔离目录、History/SQLite、性能大数据集、产品文档完成状态。
- 实现步骤：在专用 UUID 根跑多目标/中文/空格/特殊字符；以无 share-delete Handle 稳定构造 partial failure；验证 root/system/top reparse/fingerprint drift/double run/A-B-C Cancel/近自然/Core 强退；内部 Junction/Symlink 指向外部 sentinel 不变；全部通过后 flip 默认 Registry，再用默认 list/get→Preview→受控成功删除与 Cancel smoke。
- 接口、数据与错误契约：任何越界、重复启动、事实乐观化、Job/transport/reservation 残留均为门禁失败；Core 强退只证明进程树终止，不声称重启恢复结果。
- 边界与异常：测试根在创建、Preview、Run 前分别校验绝对路径和 UUID marker；cleanup 前释放 helper Handle；无法安全构造的环境不以手工删除替代证据。
- 测试要求：上述完整矩阵、默认/feature Registry、真实 UI 可见结果、进程/线程/文件/监听残留检查。
- 验证命令：Windows integration、真实 `delete-validation` Tauri、flip 后默认 Tauri smoke、完整 Rust/前端回归。
- 预期结果：默认用户第一次看到永久删除命令时，它已经通过真实宿主安全门禁。
- 完成判定：feature 与默认 smoke 均通过；只删除隔离目标；外部 sentinel 和所有非目标不变。
- 交付给下一原子的输出：可进行专项性能对照和文档收口的默认 Hero Delete。
- 停止或重新规划条件：内部链接越界、默认 smoke 与 feature 不一致、目标事实或资源清理不可靠。
- 风险等级：L3
- DDD 门禁：提交前完整实现 diff 复核必须 `PASS`。
- 计划提交信息：`feat(delete): [CMD04-WINDOWS-HOST-01] 通过永久删除宿主门禁`

## CMD04-BENCH-DOC-01 完成 Hero Delete 基准与交付收口

- 状态：todo
- 支持的验收场景：用户获得 Terminal/CmdBox 同数据集实际性能证据和完整逐项测试清单，项目准确停在 CMD-05 前。
- 唯一目标：完成权威性能设计规定的 Hero Delete Benchmark、全部质量门禁和文档状态回写。
- 当前行为与目标行为：永久删除虽已过安全门禁但没有完整专项性能数据和交付记录；完成后各数据集有可复现实测与 overhead，不虚构阈值。
- 前置条件与依赖：`CMD04-WINDOWS-HOST-01`。
- 代码定位依据：隔离数据生成/基准脚本、`docs/testing/测试与验收.md`、产品/技术/安全/性能文档、工作台、阶段记录、父级计划、根 `AGENTS.md`。
- 允许修改：安全 benchmark 工具、测试证据、唯一权威文档、计划状态和索引。
- 明确不修改：删除算法优化扩域、History/SQLite、CMD-05 或发布功能。
- 实现步骤：对相同隔离数据分别跑直接 Windows PowerShell 与 CmdBox：10,000/100,000 个 1 KiB 文件、500,000 个空文件、10,000 深层目录、中文与长路径；记录生成条件、总耗时和 overhead；执行完整门禁、危险命令/凭据/私有路径/残留扫描；回写逐项预期/实际和完成状态。
- 接口、数据与错误契约：Benchmark 每次创建新 UUID 根并核对 marker；不以生产或既有目录作数据集；没有实测就不填写或推断阈值。
- 边界与异常：磁盘、杀毒和缓存影响必须记录；任一规定数据集因资源或时长未完成，CMD-04 保持进行中并如实报告。
- 测试要求：`pnpm check`、feature/default Rust、Windows integration、contract drift、format、strict Clippy、真实宿主 smoke、完整 Benchmark、Git diff 与敏感信息检查。
- 验证命令：项目完整质量门禁和隔离 Hero Benchmark；普通源码改动不额外重建 Bundle。
- 预期结果：CMD-04 的安全、功能、取消、结果和性能证据均能按清单复现。
- 完成判定：全部自动/真实/性能证据齐全；计划八个原子 done；父级 CMD-04 done；根索引移除活动原子并明确下一推荐 CMD-05 需等待用户检查授权。
- 交付给下一单元的输出：已验证的默认 Hero Delete、Path Fingerprint、Single Flight、Target Results 和完整基准基线。
- 停止或重新规划条件：任一真实安全门禁或完整基准未通过，或文档与默认代码事实不一致。
- 风险等级：L3
- DDD 门禁：提交前最终完整 diff 复核必须 `PASS`。
- 计划提交信息：`feat(delete): [CMD04-BENCH-DOC-01] 完成永久删除交付闭环`
