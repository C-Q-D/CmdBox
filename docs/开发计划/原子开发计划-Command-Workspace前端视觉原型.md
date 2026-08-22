# Command Workspace 前端视觉原型原子开发计划

## 计划元数据

- 计划 ID：ATOMIC-UI-PROTOTYPE-001
- 类型：atomic-development
- 修订版本：1
- 状态：completed
- 父级 ID：不适用
- 创建基线：74a8f53

## 总体计划

### 交付范围

- 产品结果：把用户确认的“编辑式研究手记”方案 1 复刻为现有 Tauri 2 + React 项目中的可交互前端视觉原型，用于在真实浏览器与桌面尺寸中检查信息层级、统一 Command Workspace 和关键交互。
- 视觉真值：[Command Workspace 视觉原型](../design/Command-Workspace-视觉原型.png)，源图像素为 `1487 × 1058`。
- 支持的验收场景：用户可以在同一工作区看到命令索引、三个文件夹参数、规范化 Preview、PowerShell 预览、安全结论和永久删除动作；参数变化后旧 Preview 立即失效；任何原型交互都不产生外部命令或文件系统副作用。
- 明确排除：Tauri IPC、Rust Core 调用、真实文件选择器、真实 Preview、真实命令执行、永久删除、Execution Output、History、SQLite、Command Block CRUD 和产品 AC 完成声明。

### 现状证据与实现接缝

- `src/app/App.tsx` 与 `src/app/App.css` 当前只呈现环境准备页，没有产品界面状态。
- `src/app/App.test.tsx` 使用 Vitest、jsdom 与 Testing Library，可直接承载语义和交互回归测试。
- `src/main.tsx` 是唯一 React 挂载入口，不需要改变 Tauri 或 Vite 运行方式。
- `package.json` 已提供 `pnpm check:fast`、`pnpm check:web` 和 `pnpm check`；Vite 开发端口固定为 `1420`。
- 选中视觉只包含 CmdBox 图标和标准线性 UI 图标；品牌图标复用 `src-tauri/icons/icon.png`，通用图标使用 `@phosphor-icons/react`，不手工绘制 SVG、CSS 图标或文本符号。

### 全局约束

1. 继续使用统一 Command Workspace，不创建删除命令专属页面或命令自定义布局 DSL。
2. 原型数据必须集中定义并明确属于前端展示，不把前端模拟结果伪装成 Rust Core 事实。
3. “永久删除”在本计划内只能打开无副作用说明；源码不得导入 Tauri `invoke`、启动进程或访问文件系统。
4. 参数变化必须使 Ready Preview 失效并禁用破坏性动作；恢复演示状态后才重新显示 Ready。
5. 视觉遵守 `editorial-field-notes`：暖纸底、双字体声部、编辑网格、细线、灰蓝强信号、砖红风险信号、零渐变、零厚阴影和零圆角卡片墙。
6. 输出文本和命令 Preview 作为纯文本渲染，不使用 `dangerouslySetInnerHTML` 或自动链接。
7. 桌面以 `1487 × 1058` 源图为 QA 真值，同时保持窄桌面和小窗口不遮挡核心操作。
8. 每个原子完成针对性测试、完整前端检查、差异审查、独立提交并推送 `origin/master` 后，才能进入下一原子。

### 原子顺序

1. `UI-PROTOTYPE-01`：复刻已确认的 Command Workspace 静态视觉骨架。
2. `UI-PROTOTYPE-02`：提供无副作用的原型交互并通过浏览器视觉 QA。

### 整体验收

- `pnpm check:fast`
- `pnpm check:web`
- 应用内浏览器在 `1487 × 1058` 检查 Ready 状态和主要交互。
- 对选中原型与实现截图执行同尺寸对照，项目根 `design-qa.md` 最终必须为 `final result: passed`。
- Git 工作区干净，`master` 与 `origin/master` 一致。

### 执行模式与 Git 策略

- 执行模式：连续执行。
- 提交策略：规划、两个代码原子分别独立提交；提交标题包含计划或原子 ID。
- 同步策略：沿用项目已授权规则，每个有效提交验证通过后立即推送当前 `master` 到 `origin/master`。

## UI-PROTOTYPE-01 复刻静态 Command Workspace

- 状态：done
- 支持的验收场景：在现有 CmdBox 前端打开与选中方案 1 同构的 Ready 状态工作区。
- 唯一目标：以真实 React 结构复刻选中原型的静态信息架构和视觉层级。
- 当前行为与目标行为：当前只显示环境准备卡片；完成后显示三栏 CmdBox 桌面工作区、三个目标、Preview、安全结论和动作区。
- 前置条件与依赖：规划提交已完成；选中视觉原型已保存到项目。
- 代码定位依据：替换 `src/app/App.tsx` 和 `src/app/App.css` 的环境准备页；在 `src/features/command-workspace/` 建立实际被 App 使用的原型模块；更新 `src/app/App.test.tsx`；复用 `src-tauri/icons/icon.png`。
- 允许修改：`package.json`、`pnpm-lock.yaml`、`index.html`、`src/app/`、`src/features/command-workspace/` 及为品牌图标建立的前端静态资源。
- 明确不修改：`src-tauri/src/`、Tauri capability、IPC、开发脚本、数据库、Rust 测试和执行内核。
- 实现步骤：
  1. 安装固定主版本的 Phosphor React 图标依赖并只导入实际使用的图标。
  2. 定义静态命令摘要、三个规范化目标、Preview 和 Safety Decision 原型数据。
  3. 使用语义化 `nav`、`aside`、`main`、`section`、`table/list` 和 `pre/code` 复刻源图区域。
  4. 建立纸面、墨色、规则线、强信号和风险信号 CSS Token；用系统宋体、等宽字体和中文无衬线回退，不加载远程字体。
  5. 完成源图桌面比例和基础响应式布局，不加入本原子之外的交互逻辑。
  6. 更新页面描述和测试，证明关键区域、路径、安全状态与动作文本存在。
- 接口、数据与错误契约：不适用；所有数据都是只读原型 Fixture，不导出为后端或 IPC Contract。
- 边界与异常：窄窗口可以改变列宽或隐藏次要注释，但不得隐藏当前命令、Preview、安全结论和永久删除动作。
- 测试要求：断言命令库导航、当前标题、三个路径、`Windows PowerShell`、`预览已就绪`、`安全检查通过` 和 `永久删除` 均可通过可访问角色或文本定位；断言脚本作为纯文本存在。
- 验证命令：`pnpm test -- src/app/App.test.tsx`；`pnpm typecheck`；`pnpm build`。
- 预期结果：前端构建通过，静态 Ready 状态与视觉原型具有一致的主要区域、层级、配色与密度。
- 完成判定：生产入口不再显示环境准备卡片；所有目标内容可读；没有 Tauri、进程或文件系统调用。
- 交付给下一原子的输出：稳定的 React 视觉骨架、集中原型数据和可测试的交互接缝。
- 停止或重新规划条件：图标依赖无法安装；选中视觉无法在现有 Vite/Tauri 根组件中实现；必须修改 Rust 或 capability 才能渲染。
- 风险等级：L1
- DDD 门禁：不触发；本原子不含公共 API、权限、并发或不可逆副作用。
- 计划提交信息：`feat(ui): [UI-PROTOTYPE-01] 复刻 Command Workspace 视觉骨架`

### 执行记录

- 实际实现：已建立三栏 Command Workspace、集中原型数据、Ready Preview、安全结论与动作区；图标采用按图标直引，前端仍无 Tauri IPC、进程或文件系统调用。
- 实际验证：`pnpm test -- src/app/App.test.tsx` 通过（1 个测试）；`pnpm typecheck` 通过；`pnpm build` 通过（70 个模块，Vite 构建阶段约 720 ms）；禁用能力扫描与 `git diff --check` 通过。

## UI-PROTOTYPE-02 提供无副作用原型交互并通过视觉 QA

- 状态：done
- 支持的验收场景：用户可以搜索命令、移除目标、观察 Preview 失效、恢复演示状态，并检查永久删除动作而不会执行命令。
- 唯一目标：让已复刻的工作区成为可验证且无外部副作用的交互原型。
- 当前行为与目标行为：静态骨架只能阅读；完成后关键控件拥有可见、可访问、可测试的本地状态变化，并通过选中原型的浏览器视觉对照。
- 前置条件与依赖：`UI-PROTOTYPE-01` 已提交并推送。
- 代码定位依据：扩展 `src/features/command-workspace/` 的原型状态；在 `src/app/App.test.tsx` 增加 `fireEvent` 交互断言；使用现有 Vite 入口进行浏览器验证。
- 允许修改：`src/app/`、`src/features/command-workspace/`、必要测试、`design-qa.md`、最终实现截图、项目工作台和阶段记录。
- 明确不修改：任何 Rust 源码、Tauri IPC、真实文件选择、真实 Preview/Run/Cancel、SQLite、History 和产品 AC 状态。
- 实现步骤：
  1. 让搜索框过滤原型命令摘要，并保持当前命令的清晰选中状态。
  2. 让路径移除操作更新目标数量，立即进入“需要重新预览”状态并禁用永久删除动作。
  3. 提供“恢复原型状态”入口恢复固定的三个目标和 Ready Preview；不在前端渲染新的执行内容。
  4. 让 Ready 状态的永久删除动作只打开原型说明对话框，明确没有 IPC 和外部副作用；对话框支持关闭和键盘焦点。
  5. 完成 hover、focus-visible、窄桌面与减少动效样式，并补充交互测试。
  6. 在应用内浏览器以源图尺寸检查主状态、交互和控制台；截取实现并与源图放入同一比较输入，修复全部 P0/P1/P2 差异。
  7. 保存最终截图和 `design-qa.md`，回写当前阶段但不宣称产品功能或真实永久删除已经实现。
- 接口、数据与错误契约：所有交互只改变组件内原型状态；不得调用 `window.__TAURI__`、Tauri `invoke`、Shell、文件 API 或网络接口。
- 边界与异常：搜索无结果时保留可恢复空状态；目标为零时 Preview 保持失效；重复点击永久删除不能产生外部状态变化。
- 测试要求：断言搜索过滤；移除目标后计数变化、状态变为需重新预览且动作禁用；恢复后重新出现三个目标和 Ready；点击 Ready 动作出现“不会执行真实命令”的对话框；关闭后回到原状态。
- 验证命令：`pnpm test -- src/app/App.test.tsx`；`pnpm check:web`；应用内浏览器交互与控制台检查；视觉 QA 对照。
- 预期结果：原型关键路径可操作、无副作用，最终 QA 无 P0/P1/P2，`design-qa.md` 为 `passed`。
- 完成判定：自动测试与前端检查通过；同尺寸浏览器截图存在；QA 通过；原型仍没有后端能力。
- 交付给下一原子的输出：用户可检查的、已验证的 Command Workspace 前端视觉原型；后续可另行规划 Typed IPC 最小垂直路径。
- 停止或重新规划条件：浏览器无法捕获实现；视觉对照缺少源图；实现核心交互需要真实文件或进程权限；存在无法在本原子修复的 P0/P1/P2。
- 风险等级：L1
- DDD 门禁：不触发；破坏性动作被明确限制为无副作用原型说明。
- 计划提交信息：`feat(ui): [UI-PROTOTYPE-02] 完成安全交互与视觉验收`

### 执行记录

- 实际实现：搜索框可过滤原型 Command Block；移除或清空目标会使 Preview 和安全结论失效并禁用永久删除；恢复入口回到固定 Ready Fixture；添加文件夹与永久删除都只打开可键盘关闭的无副作用说明。
- 视觉修正：按 `1487 × 1058` 源图调整工作区两列三行网格、标题与 Runner 比例、10 条可见索引、右侧安全栏、Preview 行号、品牌脚注和动作尺寸；完整记录见项目根 `design-qa.md`。
- 浏览器验证：Chromium DevTools 完成 Ready、搜索、移除、恢复、disabled、两个说明对话框及 Escape；检查 `1240 × 900`、`980 × 900`、`500 × 812`，无水平溢出；最终 reload 后控制台无 error、warning 或 issue；Lighthouse Accessibility 100、Best Practices 100。
- 自动验证：`pnpm check` 通过，包括 4 个前端测试、TypeScript、Vite 生产构建、19 个 Rust 单元测试和 1 个 Windows 集成测试；禁用能力扫描和 `git diff --check` 通过。
- 最终证据：`docs/design/Command-Workspace-implementation-ready.png` 与 `design-qa.md`，`final result: passed`。
