# CMD-02 类型化预览与执行原子开发计划

## 计划元数据

- 计划 ID：ATOMIC-CMD-02-001
- 类型：atomic-development
- 修订版本：1
- 状态：active
- 父级 ID：CMD-02
- 创建基线：9a0bdcd

## 总体计划

- 用户结果：用户在统一 Command Workspace 中选择无破坏的 PowerShell 或 CMD 内置 Command Block，填写类型化参数，检查 Rust Core 生成的准确 Preview，再启动一次性任务并观察输出、取消和终态。
- 最终用户流程：打开 CmdBox → 选择 PowerShell/CMD 参数回显 Block → 填写 Text、Number、Boolean、Select、Folder、Folders → 选择目录或调整值 → 生成 Preview → 检查规范化摘要、完整大小、Hash 状态和执行内容 → 执行 → 查看 stdout/stderr 与终态，或运行中取消。
- 当前行为：只有固定、无参数 PowerShell 验收任务；React 不能读取 Command Block Definition，也没有通用参数、Preview 或 CMD Runner。
- 实现边界：只增加两个 Rust 内置的无破坏验收 Block；不实现用户脚本编辑、Raw Parameter、Command 持久化、Outcome Policy、破坏性路径策略或永久删除。
- 深模块 Interface：`ExecutionPlanner` 集中读取内置 Definition、严格验证、模板解析、Shell Serializer、完整执行规范 Hash、Preview 和 Run 复验；只有它能构造 `VerifiedExecution`。`ProcessLaunch` 是已验证执行到 Windows Job/Process 内核的内部接缝。
- 安全边界：React 只提交 Command Block ID、expected revision、结构化 Parameter Value、Preview Hash 和事件 Channel；不得提交脚本、可执行文件、Runner options、最终命令、工作目录旁路或 PID。
- Preview 不变量：Run 必须重新读取当前 Definition，重新验证、规范化、渲染并计算完整 Hash；不匹配时在创建临时目录、Execution、线程或进程前返回 `STALE_PREVIEW`。
- Output 与 Cancel 不变量：继续使用已验证的 Reader → Aggregator → Batch → Tauri Channel、有界 UI Buffer、Execution ID、sequence、唯一终态和 Windows Job Object 整树取消。
- 测试命令安全白名单：真实执行只允许文本回显、参数展示和短等待；禁止删除、覆盖、移动、安装、网络访问、注册表、系统配置或其他高影响命令。目录参数只能指向项目内专用测试目录或系统临时目录，命令不得修改其内容。
- CMD 当前契约：确定的 System32 `cmd.exe`；UTF-8 无 BOM、CRLF `.cmd`；ASCII `chcp 65001` 前导；参数经私有 UTF-16 Environment 和 delayed expansion 注入，不写入 Batch 源码；拒绝 NUL/CR/LF、8191 字符超限和二次命令解析上下文。
- 并行与 Git：用户明确允许项目内 `.worktree/` 并行。`CMD02-LAUNCH-01` 与 `CMD02-TEMPLATE-01` 可在独立 worktree 同时实现，主分支按原子编号审查并合并；其余按依赖顺序。每个原子独立验证、提交并立即推送 `origin/master`。
- 整体回归：各原子窄测试；最后执行 `pnpm check`、真实 Windows PowerShell/CMD 集成测试、真实 Tauri 用户流程、响应式/可访问性/控制台检查和无危险命令审计。
- 计划质疑：L3 隔离复核结果为 `PASS`；原子顺序、私有 `VerifiedExecution`、CMD Environment 注入、窄 IPC 和统一 UI 路径未发现阻断当前最小交付的问题。

## 原子依赖

```text
CMD02-LAUNCH-01 ─┐
                 ├→ CMD02-PREVIEW-01 → CMD02-PS-RUN-01 → CMD02-CMD-01
CMD02-TEMPLATE-01┘                                      │
                                                       ├→ CMD02-UI-CONTRACT-01
                                                       │        ↓
                                                       │  CMD02-UI-FORM-01
                                                       │        ↓
                                                       │  CMD02-UI-PREVIEW-01
                                                       │        ↓
                                                       └→ CMD02-UI-RUN-01
```

## CMD02-LAUNCH-01 收敛受管脚本启动接缝

- 状态：done
- 支持的验收场景：现有固定 PowerShell 验收任务继续自然结束、输出、取消和整树清理，同时进程内核不再依赖具体 Shell 类型。
- 唯一目标：让 `ManagedProcess` 只消费一个已解析的 `ProcessLaunch`。
- 当前行为与目标行为：当前 `ManagedProcess::prepare` 直接接收 `WindowsPowerShellRunner` 与 `PowerShellArtifact`；完成后 Runner 解析、最终脚本字节、临时脚本租约和 Win32 启动参数在进程内核外收敛，现有 CMD-01 行为保持等价。
- 前置条件与依赖：CMD-01 已完成；无本计划内依赖。
- 代码定位依据：`execution/artifact.rs`、`process/windows/runner.rs`、`process/windows/managed_process.rs`、`execution/session.rs` 和 `tests/managed_process_windows.rs`。
- 允许修改：上述 Rust 文件及其直接测试；现有 `execution/output.rs` 测试和 `bin/cmdbox-job-test-helper.rs` 只允许机械迁移到新启动值。
- 明确不修改：Tauri IPC、React、Command Block、Typed Parameter、CMD Runner 和用户可见行为。
- 实现步骤：把最终编码后的脚本字节与脚本类型建模为受控值；临时文件固定扩展名、随机目录、Flush、Hash 复验和 RAII 清理；建立字段私有的 `ResolvedRunner`/`ProcessLaunch`；使 `ManagedProcess` 只负责 CreateProcessW、Pipe、Job、Resume 和终止；用现有固定入口构造等价 PowerShell Launch。
- 接口、数据与错误契约：最终 Artifact Hash 覆盖实际落盘字节；随机临时路径不进入执行语义；IPC 无法构造 `ProcessLaunch`。
- 边界与异常：工作目录仍需绝对且存在；Hash 不匹配不得 CreateProcess；失败路径只清理当前唯一临时目录。
- 测试要求：BOM/字节 Hash、篡改拒绝、精确清理、Runner 固定参数、自然结束、非零 Exit、Cancel、慢消费和 Core 强退全部不回归。
- 验证命令：相关 Rust 模块测试、`cargo test --manifest-path src-tauri/Cargo.toml`、`cargo fmt --manifest-path src-tauri/Cargo.toml --check`。
- 预期结果：CMD-01 的可观察行为不变，Shell 差异从 Win32 Job/Process 实现中移除。
- 完成判定：全部既有 Rust/Windows 测试通过，diff 不包含新产品能力。
- 交付给下一原子的输出：可承载 PowerShell 或 CMD 最终脚本字节的 `ProcessLaunch`。
- 停止或重新规划条件：泛化要求改变 Job、Output、Cancel 或现有终态语义。
- 风险等级：L2
- DDD 门禁：提交前一轮限定范围复核必须 `PASS`。
- 计划提交信息：`refactor(core): [CMD02-LAUNCH-01] 收敛受管脚本启动接缝`

### 执行记录

- 实际验证：Artifact 4 项、Runner 3 项、ManagedProcess 4 项、Session 6 项窄测通过；完整 Rust 单元测试 26 项、Windows 强退集成 1 项通过；`cargo fmt --check`、严格 Clippy、`git diff --check` 通过；限定提交前复核为 `PASS`。
- 计划偏差：现有 Output 真实进程测试和 feature-gated Job helper 直接调用旧入口，为保持编译与原测试语义，对两个调用方做了仅参数构造的机械迁移。

## CMD02-TEMPLATE-01 建立类型化参数与受限模板

- 状态：done
- 支持的验收场景：六类 Parameter Value 由 Rust 按当前 Definition 严格校验，模板只能用已冻结的三类节点表达。
- 唯一目标：从内置 Command Block Definition 与结构化值生成确定、有序的模板语义。
- 当前行为与目标行为：当前后端没有 Command Block 或模板；完成后具有两个无破坏内置 Definition、严格参数校验和 `value/if/each` AST，但尚不生成可执行 Preview。
- 前置条件与依赖：无，可与 `CMD02-LAUNCH-01` 在独立 worktree 并行。
- 代码定位依据：新增 `execution/command.rs`、`execution/parameter.rs`、`execution/template.rs`，更新 `execution/mod.rs`。
- 允许修改：新增纯计算模块和直接单元测试。
- 明确不修改：进程、Artifact、IPC、React、路径安全判断、持久化和任意用户模板编辑。
- 实现步骤：定义两个固定正常风险 Built-in；支持 Text、Number、Boolean、Select、Folder、Folders；按 Definition 顺序严格验证全部且仅有声明 key；Folder/Folders 只规范化绝对路径文本并按 `mustExist` 检查目标根，不读取目录内容；解析 `{{value}}`、`{{#if key}}...{{/if}}`、`{{#each key}}...{{this}}...{{/each}}`；拒绝未定义变量、错误类型、非法嵌套、Raw 或表达式。
- 接口、数据与错误契约：稳定错误包含参数 key 和错误码，不包含本机私有路径细节；规范化结果使用确定顺序，不依赖 HashMap 迭代。
- 边界与异常：Number 必须有限并满足 min/max/step；Select 必须来自固定 options；Folders 满足 min/maxItems；未知/缺失值稳定拒绝。
- 测试要求：六类正常值、中文/空格/单引号、多路径、未知 key、缺失、错误 JSON 类型、范围和选项错误；模板嵌套、未定义变量和非法 `this`。
- 验证命令：新增模块窄测试、`cargo test --manifest-path src-tauri/Cargo.toml`。
- 预期结果：同一 Definition 与 Value 始终产生相同的规范化参数和 AST 语义。
- 完成判定：纯计算测试覆盖当前成功与常见错误路径，不创建文件或进程。
- 交付给下一原子的输出：内置 Definition、规范化 Parameter 和受限模板 AST。
- 停止或重新规划条件：当前三个模板节点不能表达两个安全验收 Block。
- 风险等级：L2
- DDD 门禁：提交前一轮限定范围复核必须 `PASS`。
- 计划提交信息：`feat(core): [CMD02-TEMPLATE-01] 建立类型化参数与受限模板`

### 执行记录

- 实际验证：Parameter 9 项、Template 7 项、Command 2 项窄测通过；完整 Rust 单元测试 42 项、Windows 集成 1 项通过；`cargo fmt --check` 和 `git diff --check` 通过。限定复核发现大数 Number 步长容差可错误接受半步值，红测复现后将容差限制为严格小于半步，回归测试通过，其余契约无阻断问题。

## CMD02-PREVIEW-01 生成可信 PowerShell Preview

- 状态：done
- 支持的验收场景：用户提交 PowerShell Block 参数后得到 Rust 规范化摘要、可读脚本和绑定完整执行规范的 Hash。
- 唯一目标：以深模块 `ExecutionPlanner` 统一 Preview 与 Run 复验计算。
- 当前行为与目标行为：已有参数语义和受管启动值但没有 Preview；完成后 Planner 能生成 PowerShell Preview，并以同一内部路径产出私有 `VerifiedExecution`。
- 前置条件与依赖：`CMD02-LAUNCH-01`、`CMD02-TEMPLATE-01`。
- 代码定位依据：新增 `execution/serializer.rs`、`execution/planner.rs`、必要的 `execution/spec.rs`，更新 `execution/mod.rs` 和直接测试。
- 允许修改：Planner 及内部 Serializer/Spec、必要 Cargo 直接依赖和纯计算测试。
- 明确不修改：IPC、React、Process 启动、CMD Serializer、Safety Delete 和持久化。
- 实现步骤：PowerShell 单引号 literal serializer；Renderer 生成含 UTF-8 BOM 的最终字节；建立有界 Parameter Summary 与 Preview Text；Canonical Spec 使用带 schema 版本的确定性 length-prefixed 编码，覆盖 Block ID/revision、Runner executable/options、最终 Artifact Hash、规范化参数、工作目录、显式环境、安全/Outcome policy version；`verify_run` 全量重建并比较 Hash 后才返回字段私有的 `VerifiedExecution`。
- 接口、数据与错误契约：外部 Interface 只暴露 list/get Definition、preview、verify_run；不公开 Parser/Validator/Serializer 浅接口；Preview Text 截断不改变完整 Hash。
- 边界与异常：Hash 使用完整最终字节；路径显示可读但错误不得泄露本机内部路径；旧 revision 与旧 hash 使用不同稳定错误。
- 测试要求：所有 Hash 组成分别改变后旧 Hash 失效；Map 输入顺序不影响 Hash；单引号成对转义；中文、空格、多路径和 if/each 输出确定；展示截断仍覆盖完整 Artifact。
- 验证命令：Planner/Serializer 窄测试、完整 Rust 单测。
- 预期结果：Preview 和 Run 共享唯一计算实现，调用方无法构造已验证执行。
- 完成判定：Hash 覆盖矩阵和 serializer 测试通过，尚不启动任何进程。
- 交付给下一原子的输出：`PreviewCommandResponse` 与 `VerifiedExecution`。
- 停止或重新规划条件：Run 需要复制 Preview 计算逻辑，或 Verified 值能被 IPC 直接构造。
- 风险等级：L3
- DDD 门禁：提交前一轮限定范围复核必须 `PASS`。
- 计划提交信息：`feat(core): [CMD02-PREVIEW-01] 生成可信 PowerShell Preview`

### 执行记录

- 实际交付：新增唯一 `ExecutionPlanner`、PowerShell 单引号 Serializer、有 Schema Version 的 length-prefixed Canonical Execution Spec、规范化有界摘要、完整 Artifact 大小与 Preview Hash；Run 复验先区分 revision conflict，再以同一路径全量重建并比较 Hash。内部 Definition/模板不进入 list/get DTO，请求拒绝未知旁路字段；`VerifiedExecution` 只能经 crate 内消费入口生成字段私有的 `ProcessLaunch`。
- 实际验证：Planner 9 项、Serializer 3 项、Canonical Spec 3 项及 sibling 消费边界测试通过；完整 Rust 单元测试 60 项、Windows 集成 1 项通过；`cargo fmt --check`、strict Clippy 和 `git diff --check` 通过。限定复核发现授权值无法被 Session 消费、list/get 会暴露内部模板，均按最小接口修正并加入回归测试；本原子未新增临时脚本、线程或进程测试。

## CMD02-PS-RUN-01 接通 PowerShell Preview 执行

- 状态：done
- 支持的验收场景：PowerShell 内置 Block 从窄 IPC Preview 后执行，显示安全参数回显并可取消。
- 唯一目标：让现有 Execution Core 只启动 `VerifiedExecution`。
- 当前行为与目标行为：当前 Tauri 只能启动固定脚本；完成后 list/get/preview/run/cancel 均按 Command Block 业务命名，固定脚本旁路删除。
- 前置条件与依赖：`CMD02-PREVIEW-01`。
- 代码定位依据：`execution/session.rs`、`ipc/execution.rs`、`ipc/mod.rs`、`lib.rs` 及直接 IPC/Windows 测试。
- 允许修改：上述后端接线和测试。
- 明确不修改：React、CMD Runner、Outcome Policy、持久化或破坏性命令。
- 实现步骤：注册窄 list/get/preview/run/cancel Commands；Run Request 只含 ID/revision/values/hash/Channel；Planner 在任何外部副作用前 verify；`ExecutionManager::start` 只接受 Verified 值；删除 `start_fixed_execution` 和 `start_fixed_powershell` 产品旁路；事件转发复用既有实现。
- 接口、数据与错误契约：错误码至少区分 Validation、Block Not Found、Revision Conflict、Stale Preview、Runner、Artifact、Process 和 IPC；公开消息不泄露底层路径；Cancel 仍只接受 Execution UUID。
- 边界与异常：重复启动防线沿用 starting UI 与 Manager；Channel 建立失败请求 Job 清理；旧 Hash 不创建 `%TEMP%/CmdBox` 新目录或 Active Execution。
- 测试要求：IPC 字段白名单、无脚本/PID；PowerShell 中文/空格/单引号/multi path/if/each 真实执行；参数/revision/options/cwd/environment/policy 变化 stale；自然结束、Cancel、Channel 断开、Output 文本不回归。
- 验证命令：窄 IPC/Rust 测试、完整 Rust 测试和安全 PowerShell Windows 集成测试。
- 预期结果：固定旁路消失，唯一启动路径为 Planner → VerifiedExecution → ExecutionManager。
- 完成判定：真实 PowerShell 只回显/短等待并完成或取消，旧 Hash 证明没有进程副作用。
- 交付给下一原子的输出：通用、窄且可复用的 Preview/Run IPC 与 Session 路径。
- 停止或重新规划条件：必须保留可绕过 Preview 的公开启动入口。
- 风险等级：L3
- DDD 门禁：提交前一轮限定范围复核必须 `PASS`。
- 计划提交信息：`feat(core): [CMD02-PS-RUN-01] 接通 PowerShell Preview 执行`

### 执行记录

- 实际交付：Tauri 后端只注册 list/get/preview/run/cancel 五个 Command Block 命令；Run 在任何 Artifact、进程、Active 或转发线程副作用前完成 `verify_run`，唯一生产启动入口消费字段私有的 `VerifiedExecution`。固定脚本旁路已删除；公开同步错误使用稳定码和脱敏文案，Channel 断开不取消已经启动的 Execution。
- 实际验证：IPC 9 项、Session 7 项、完整 Rust 单元测试 64 项和 Windows Job Object 集成测试 1 项通过；普通应用构建、`cargo fmt --check`、strict Clippy、`git diff --check` 与危险命令文本审计通过。真实 PowerShell 测试只运行参数回显、有限输出、非零退出和短等待/取消；L3 隔离复核结论为 `PASS`。
- 计划偏差：当前 Windows 主机上的 Tauri `test` feature 探针因测试 EXE 缺少 Common Controls v6 清单，在 Rust 测试主体运行前触发 `TaskDialogIndirect` 装载错误；诊断探针与临时目录已移除，正式应用不受影响。Tauri Command 层改用生产 IPC Adapter 直接测试，并由普通 Tauri 应用构建验证注册表。

## CMD02-CMD-01 增加确定性 CMD 执行适配

- 状态：done
- 支持的验收场景：CMD 内置 Block 对中文、空格、单引号和 Shell 元字符做字面参数回显，条件/循环正确且不执行注入内容。
- 唯一目标：让既有 Planner/VerifiedExecution/Session 支持确定性 CMD Script Artifact。
- 当前行为与目标行为：已有 PowerShell 纵向路径；完成后第二个内置 CMD Block 通过同一业务 Interface 工作。
- 前置条件与依赖：`CMD02-PS-RUN-01`。
- 代码定位依据：Runner、Artifact、Serializer、Planner、ManagedProcess 环境块和 Windows 集成测试。
- 允许修改：CMD Adapter 所需后端文件、Win32 feature 和测试 helper/集成测试。
- 明确不修改：React、任意 CMD 编辑、文件副作用、自动编码猜测和其他 Runner。
- 实现步骤：System32 解析 `cmd.exe`；最终 `.cmd` 使用 UTF-8 无 BOM、CRLF、ASCII `chcp 65001` 与 `setlocal` 前导；固定 `/D /Q /A /E:ON /V:ON /S /C`；Artifact 路径和参数通过 Rust 构建的私有 UTF-16 Environment block 与 delayed expansion 进入脚本；CMD `/C` 使用固定 raw command tail，其他 Runner 参数继续使用标准 Windows quoting；模板上下文拒绝二次解析结构。
- 接口、数据与错误契约：私有环境变量名和值、Runner flags 和最终 Artifact 字节进入 Canonical Spec；配置环境不能覆盖内部保留名；Process 使用 `CREATE_UNICODE_ENVIRONMENT`。
- 边界与异常：拒绝 NUL/CR/LF、单变量或展开物理命令行达到 8191 字符、`CALL`、嵌套 `cmd /C|K` 和 `for /f ('...')` 插值上下文；不承诺自定义 argv parser 或不遵守 CP65001 的外部程序。
- 测试要求：中文、日文、Emoji、空格、单双引号、`& % ^ ! ( ) < > |`、反斜杠、空值、if/each、stdout/stderr、含特殊字符 Artifact 路径；注入标记不得成为额外输出或文件；当前 Windows 真实集成只运行回显/短等待。
- 验证命令：CMD serializer/Runner/Artifact 单测、Windows 集成测试、完整 Rust 回归。
- 预期结果：CMD 参数按字面到达，Shell 语法只来自固定模板节点。
- 完成判定：当前真实 Windows 编码与元字符矩阵全部通过且无额外副作用。
- 交付给下一原子的输出：Rust serde 已冻结的 PowerShell/CMD Definition、Preview、Run 与错误契约。
- 停止或重新规划条件：任一规定字符会改变命令结构、输出无法确定解码或必须允许参数写入 Batch 源码。
- 风险等级：L3
- DDD 门禁：提交前一轮限定范围复核必须 `PASS`。
- 计划提交信息：`feat(core): [CMD02-CMD-01] 增加确定性 CMD 执行适配`

### 执行记录

- 实际交付：新增确定解析 `cmd.exe`、`chcp.com` 和 `SystemRoot` 的 `CmdRunner`，以 UTF-8 无 BOM `.cmd`、固定 raw `/S /C` tail 和完全替换的 UTF-16 Environment block 执行第二个类型化回显 Built-in。非空参数只进入确定命名的私有环境，空值静态渲染；严格 AST/物理行 allowlist 只接受单次解析的 `echo(` 行，拒绝控制字符、二次解析结构和达到 8191 UTF-16 单元的值或物理行。Canonical Spec Schema 2 已覆盖 Runner、raw tail、最终 Artifact Hash、显式/固定/私有环境和全部既有执行事实；随机 Artifact 路径仅作为 launch-only 值。
- 实际验证：CMD 窄测 13 项、完整 Rust 单元测试 78 项和 Windows Job Object 集成测试 1 项通过；普通应用构建、`cargo fmt --check`、strict Clippy、`git diff --check` 与危险命令审计通过。真实 CMD 仅执行回显和受控非零退出，已覆盖中文、日文、Emoji、单双引号、反斜杠、空值、`& % ^ ! ( ) < > |` 及含特殊字符 Artifact 路径；输出、Exit Code、Active Execution、目标目录和 Artifact 清理均精确断言。L3 隔离复核结论为 `PASS`。
- 计划偏差：完全替换环境的真实红测证明 CMD 分派 Batch 仍需要 `SystemRoot`，因此从 System32 确定推导该值并纳入固定环境和 Canonical Hash；`/S` 同时纳入固定 Runner options，确保双层引号 raw tail 的处理语义被冻结。既有强退 helper 无法运行 RAII 清理的问题也在本原子内补齐了 UUID 子目录回报、范围验证和测试侧精确清理。

## CMD02-UI-CONTRACT-01 接入前端契约与目录选择

- 状态：done
- 支持的验收场景：前端能读取后端 Definition/Preview/Run 类型并通过原生对话框选择一个或多个目录，但尚不改工作区主流程。
- 唯一目标：建立 TypeScript Gateway 和可注入 `FolderPicker` 两个窄接缝。
- 当前行为与目标行为：当前 Gateway 只启动固定任务；完成后由 Rust serde 稳定生成 TypeScript Contract，官方目录对话框只开放 Open 能力。
- 前置条件与依赖：`CMD02-CMD-01` 的 Rust serde 契约已冻结。
- 代码定位依据：Rust serde DTO 与 TypeScript Contract 生成入口、`src/generated/`、`execution-gateway.ts/test.ts`、新增 folder picker、`package.json`、`Cargo.toml`、`lib.rs`、Capability。
- 允许修改：前端契约、Gateway、Dialog Plugin 最小注册与测试。
- 明确不修改：Command Workspace 状态机、表单 UI、Preview 展示和 Execution 行为。
- 实现步骤：从 Rust serde Struct/Enum 生成 discriminated unions 并校验生成文件无漂移；Gateway 提供 list/get/preview/run/cancel；Run 内部创建 Channel；官方 Dialog Plugin 只开放 `dialog:allow-open`；FolderPicker 单选/多选取消返回 null；浏览器环境不可用。
- 接口、数据与错误契约：请求不能出现 script/executable/PID/options；错误只接受稳定白名单字段；未知拒绝值不回显对象。
- 边界与异常：不开放 save/message/fs/shell/opener；选择路径不在前端读取或规范化。
- 测试要求：五个 IPC 命令名与参数精确、Channel 创建、浏览器降级、错误白名单、Folder/Folders 选择和取消。
- 验证命令：Contract 生成/漂移检查、Gateway/Picker Vitest、TypeScript typecheck、Rust plugin 编译检查。
- 预期结果：前端得到真实业务 Contract，但现有 CMD-01 工作区仍可编译运行。
- 完成判定：契约测试证明没有任意执行或文件访问入口。
- 交付给下一原子的输出：`CommandExecutionGateway`、Parameter types 和 `FolderPicker`。
- 停止或重新规划条件：官方插件必须开放超出目录选择的权限。
- 风险等级：L2
- DDD 门禁：提交前一轮限定范围复核必须 `PASS`。
- 计划提交信息：`feat(ui): [CMD02-UI-CONTRACT-01] 接入命令契约与目录选择`

### 执行记录

- 实际验证：Rust serde DTO 通过 `ts-rs 12.0.1` 固定根白名单生成唯一 `src/generated/contracts.ts`，普通测试只在独占临时目录生成并执行只读漂移比较；Gateway/FolderPicker 窄测 11 项、完整前端测试 27 项、Rust 单元测试 80 项与 Windows 集成测试 1 项通过，`pnpm typecheck`、Vite 构建、契约漂移、`cargo fmt --check`、strict Clippy、普通应用构建和 `git diff --check` 均通过。
- 权限与边界：正式前端只保留 list/get/preview/run/cancel 五个 IPC 命令，只有 Run 创建 Channel；旧固定命令名已从生产源码移除。Dialog Plugin 仅开放 `dialog:allow-open`，未开放 save/message/fs/shell/opener，Picker 不读取、规范化、去重或重排路径。
- 计划偏差：总体技术设计要求 Rust serde 是前端 Contract 单一真值，因此没有手写平行 TypeScript 模型；改用仅测试构建生效的生成与漂移门禁。首次限定复核发现测试注入路径仍可触达已删除旧命令，删除该生产旁路并补五命令回归后复核结论为 `PASS`。主工作树的全新 Cargo Target 回归还发现 `pnpm check` 未启用 Windows 集成测试所需 helper feature；现已让完整门禁显式启用该 feature，并让普通无 feature 的 `cargo test` 不误运行缺少 helper 的测试。连续回归同时复现了慢消费者测试依赖固定 2 秒等待的时序假设，改为在全程零消费条件下限时等待 Session 关闭后再验证容量、终态和丢弃字节，保留且强化原背压不变量。

## CMD02-UI-FORM-01 渲染统一类型化参数表单

- 状态：in_progress
- 支持的验收场景：用户可切换两个真实 Built-in，并按 Definition 填写六类参数。
- 唯一目标：让 Command Workspace 由后端 Summary/Definition 驱动统一 Parameter Form。
- 当前行为与目标行为：当前索引是静态 Fixture 且只有固定项可用；完成后只显示后端真实 Built-in，使用同一表单映射，不执行或 Preview。
- 前置条件与依赖：`CMD02-UI-CONTRACT-01`。
- 代码定位依据：`CommandWorkspace.tsx`、`command-data.ts`、新增 `ParameterForm.tsx`、`App.css`、`App.test.tsx`、`package.json` 和 `pnpm-lock.yaml`。
- 允许修改：Definition/选择/表单相关 React、CSS、测试，以及技术设计已指定但尚未安装的 React Hook Form 与 Zod 精确依赖。
- 明确不修改：Preview、Run/Cancel/Output 内核、命令专属页面或布局 DSL。
- 实现步骤：安装并精确锁定 React Hook Form 与 Zod；加载真实 Summary 并选中第一项；切换时按 generation 丢弃迟到 Definition；用 Definition 默认值初始化 React Hook Form；由 Definition 构造只用于即时 UX 的 Zod 校验，Rust 仍是权威；固定 switch 映射六种控件；Folder/Folders 调用 Picker；参数变化保持 configuring 且不制造虚假 Preview；Execution 活跃时禁用切换和输入。
- 接口、数据与错误契约：Parameter 顺序、label、description、required 和约束只来自 Definition；前端可做即时 UX 校验但 Rust 仍是权威。
- 边界与异常：对话框取消不改变值；Folders 可移除；搜索只过滤真实 Summary；不显示虚假总数或未接入命令。
- 测试要求：两个 Block 切换、六控件、默认值/约束、Picker 取消/选择/移除、搜索、迟到 Definition、运行中只读和参数变化失效。
- 验证命令：相关 Vitest、typecheck、Vite 构建。
- 预期结果：同一 Workspace 能配置 PowerShell/CMD Block，不含 Shell 专属 React 分支。
- 完成判定：表单可访问且保持现有 editorial-field-notes 网格和响应式。
- 交付给下一原子的输出：当前 Definition 与结构化 Parameter Values。
- 停止或重新规划条件：需要命令专属 React 组件才能完成两条 Block。
- 风险等级：L2
- DDD 门禁：提交前一轮限定范围复核必须 `PASS`。
- 计划提交信息：`feat(ui): [CMD02-UI-FORM-01] 渲染统一类型化参数表单`

## CMD02-UI-PREVIEW-01 呈现 Rust Preview 与失效状态

- 状态：pending
- 支持的验收场景：用户生成并检查当前参数对应的规范化 Preview，参数或命令变化后旧 Preview 立即不可执行。
- 唯一目标：完成 Command Workspace 的 configuring → previewing → ready 流程。
- 当前行为与目标行为：表单已有结构化值但没有 Preview；完成后只显示 Rust 返回的摘要、文本、大小、截断、安全和 Hash 状态。
- 前置条件与依赖：`CMD02-UI-FORM-01`。
- 代码定位依据：`CommandWorkspace.tsx`、Preview 样式、`App.test.tsx`。
- 允许修改：Preview 状态、展示、动作和测试。
- 明确不修改：Execution Channel、Cancel、Outcome 或前端 Shell 处理。
- 实现步骤：Preview 请求携带 ID/revision/完整 Values；每次修改/切换递增 generation 并清除 ConfirmedPreview；只接受当前 ID/revision/generation 响应；保存不可变参数快照与完整 Hash；展示规范化 Summary、Preview Text、完整大小和截断；normal/notApplicable 省略危险区；blocked 不进入 ready。
- 接口、数据与错误契约：前端不计算 Hash、不使用展示文本授权 Run、不用原始路径伪造规范化摘要；Preview 文本永远以 `<pre>` 文本呈现。
- 边界与异常：迟到响应丢弃；字段错误定位对应参数；Stale/Revision 错误回 configuring；无参数 Block 自动 Preview 但仍需用户明确 Run。
- 测试要求：成功、字段错误、blocked、truncated、参数失效、切换失效、乱序响应、HTML/ANSI/URL 纯文本和完整 Hash 快照。
- 验证命令：相关 Vitest、typecheck、Vite build。
- 预期结果：只有当前 Rust Preview 能启用后端动作文案。
- 完成判定：无法通过参数竞态、旧响应或截断文本运行旧内容。
- 交付给下一原子的输出：不可变 `ConfirmedPreview` 和 ready 工作区。
- 停止或重新规划条件：需要在前端重现 Rust 规范化或安全判断。
- 风险等级：L2
- DDD 门禁：提交前一轮限定范围复核必须 `PASS`。
- 计划提交信息：`feat(ui): [CMD02-UI-PREVIEW-01] 呈现可信 Preview`

## CMD02-UI-RUN-01 完成类型化命令宿主闭环

- 状态：pending
- 支持的验收场景：用户从当前 Confirmed Preview 运行 PowerShell/CMD，观察字面参数输出、自然结束或取消，并得到稳定反馈。
- 唯一目标：把 ready Preview 接到既有 Execution Stream 并取得完整真实宿主证据。
- 当前行为与目标行为：已有表单和 Preview；完成后 Run/Output/Cancel/终态形成完整 CMD-02 产品闭环。
- 前置条件与依赖：`CMD02-UI-PREVIEW-01`。
- 代码定位依据：`CommandWorkspace.tsx` 的 CMD-01 run generation/Channel/Output/Cancel 逻辑、Gateway、App tests、视觉 QA 和项目状态文档。
- 允许修改：Execution 接线、动作状态、结果展示、测试、必要视觉调整和权威状态/验收文档。
- 明确不修改：Outcome Policy、History、持久化、永久删除或任何危险 Command Block。
- 实现步骤：Run 只读取 ConfirmedPreview 的 ID/revision/完整 Values/完整 Hash；starting 防双击；保留 Execution ID/sequence/generation/唯一终态认证、有界 Output 和 Cancel；finished 后再次运行必须重新 Preview；真实宿主分别运行 PowerShell/CMD 回显 Block并测试 Cancel；执行视觉、响应式、键盘和控制台检查；同步父计划、工作台、阶段记录、测试与验收和活动索引。
- 接口、数据与错误契约：`STALE_PREVIEW` 清除 Preview 且不创建 Execution；按钮、计时器和 stdout 不能推断 Lifecycle；Output 不解释 HTML/ANSI/URL。
- 边界与异常：运行中禁止切换/修改；迟到 Cancel 不倒退终态；Browser 无 Tauri 时 Gateway/Picker/窗口动作保持安全降级。
- 测试要求：双击 Run 一次调用、Run 快照、Stale、Started 前事件缓存、Execution ID/sequence 认证、自然结束、Cancel 竞态、512 KiB 上限、两 Runner 字符矩阵、目录值只回显不修改、无危险命令审计。
- 验证命令：前端窄测试、`pnpm check`、真实 Windows 集成测试、`dev.cmd -Detached` Tauri 流程、响应式/可访问性/控制台检查、`stop.cmd` 清理后重新启动供用户检查。
- 预期结果：CMD-02 在当前真实 Windows/Tauri 完成，PowerShell/CMD 都只执行无副作用内置任务。
- 完成判定：最终测试清单逐项记录操作、预期与实际；所有自动和真实宿主门禁通过；仓库文档只描述已证实行为。
- 交付给下一原子的输出：供 `CMD-03` 使用的 Typed Preview/Run/Execution 基础。
- 停止或重新规划条件：真实 CMD 字符改变命令结构、目录内容被修改、旧 Preview 启动进程或宿主无法完成闭环。
- 风险等级：L3
- DDD 门禁：提交前一轮限定范围复核必须 `PASS`。
- 计划提交信息：`feat(ui): [CMD02-UI-RUN-01] 完成类型化命令宿主闭环`
