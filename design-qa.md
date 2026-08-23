# Command Workspace 视觉 QA

## 当前结论

`CMD-02` 当前界面已从无副作用视觉原型进入真实 Rust Contract 驱动的 Command Workspace：默认应用只显示 PowerShell 与 CMD 两个正式参数回显 Built-in，统一表单支持 Text、Number、Boolean、Select、Folder、Folders，Preview、Run、Output、Cancel 和唯一终态均使用同一套工作区。

当前视觉、响应式、单标题栏和已覆盖键盘路径通过；没有发现会阻断当前双 Built-in 流程的 P0、P1 或 P2 问题。本轮没有取得独立 WebView console attachment，因此不声称 fresh console 为 0 error/warning/issue；具体证据边界见“控制台与运行时证据”。

## 当前对照基线

- 视觉语言来源：`docs/design/Command-Workspace-视觉原型.png`，`1487 × 1058`。它记录用户确认的 `editorial-field-notes` 视觉语法和最初 Hero Delete 原型，继续作为布局、字体、色彩和编辑式证据层级的视觉基线，不再作为当前业务内容真值。
- 当前实现截图：`docs/design/Command-Workspace-implementation-ready.png`，`1487 × 1058`。当前截图是默认构建中的 PowerShell 参数回显 Preview Ready 状态，只截取 Tauri 客户区；Folder/Folders 使用 `C:\Users\Public\Documents` 与 `C:\Users\Public\Downloads` 中性公共目录，不包含本机 Workspace 路径或底层终端背景。
- 单标题栏证据：`docs/design/Command-Workspace-Tauri-single-titlebar.png`。当前 Tauri 主窗口仍只有一层 CmdBox 自定义标题栏。
- 当前内容真值：Rust `list/get/preview/run/cancel` Contract、两个正式 Built-in Definition 和 `docs/testing/测试与验收.md` 的 CMD-02 清单。
- density normalization：源视觉与当前 Ready 截图均为 1:1 像素证据；响应式检查另按 CSS viewport 记录，不把设备外框计入页面宽度。

## 当前默认界面事实

- 命令索引严格显示 2 个正式 Built-in：`PowerShell 参数回显` 与 `CMD 参数回显`。feature-only 短等待 Definition 在默认开发和 release 中不可见。
- 标题、描述、Runner、Origin、Risk、revision 和参数 Definition 均来自 Rust Summary/Details；React 不保留命令专属假数据。
- 两个命令复用同一个六类参数表单。切换命令按新的 Definition generation 完整重建表单，不继承旧命令的同 key 值。
- Preview Ready 状态显示 Rust 规范化参数摘要、完整大小、Runner 和 64 个十六进制字符（256 bit）的 Execution Spec Hash。普通命令且 Safety 不适用时不伪造破坏性安全区域。
- Run 只消费一次当前 Confirmed Preview；自然结束、Cancel 或拒绝后都必须重新 Preview，界面不会保留可再次运行的旧 Hash。
- Execution 区只显示认证 UUID、纯文本 stdout/stderr、Lifecycle、Exit Code、耗时与唯一终态；当前直接渲染边界为 512 KiB 和 2048 个非空 Chunk。
- 右侧注释栏分别陈述 Definition 身份、可信 Preview 与 Execution 内核，不把未来的 Outcome、History、持久化或永久删除描述成当前能力。

## 当前视觉 Findings

- 当前没有可执行的 P0、P1 或 P2 视觉差异。
- [P3] Windows 系统中文字体回退存在轻微字面差异。
  - Location：大标题、侧栏正文和小号等宽标签。
  - Evidence：视觉源图是生成的印刷式中文排版；实现使用本机宋体、系统无衬线和 Cascadia Mono 等回退，层级与占位接近，但不同 Windows 字体安装会产生轻微字宽和抗锯齿差异。
  - Impact：不改变布局、层级、可读性或操作。
  - Fix：当前无需修复；若未来引入受许可的内置字体资产，再锁定跨机器字形。

## 必查表面

- Fonts and typography：大标题使用中文宋体声部，UI 正文使用系统无衬线，证据与技术事实使用等宽字体；标题、标签、正文、参数值和代码的层级清楚。
- Spacing and layout rhythm：暖纸底、细线分区、左侧全局导航、命令索引、主工作区、右侧注释栏和底部动作区延续原型的编辑式研究手记语法，没有回到卡片墙。
- Colors and visual tokens：深墨、灰蓝证据、砖红风险和细灰规则线由统一 CSS Token 控制；没有渐变和厚阴影。
- Image quality and assets：品牌标记复用 `src-tauri/icons/icon.png`；其余 UI 图标使用同一 Phosphor 线性图标家族，没有文本符号、手绘 SVG、拉伸图片或占位图。
- Copy and content：当前文案围绕真实 Definition、六类参数、可信 Preview、Execution 与 Output；不再展示“共 24 个命令块”、三个删除目标、永久删除动作或固定诊断脚本等原型假数据。
- Trust presentation：Hash、UUID、revision、Runner 和 Output 上限在相应证据区域出现；React 不根据展示文本推断后端事实。

## 当前响应式证据

| Viewport | 实际结果 | 可见 focusable | 说明 |
|---|---|---:|---|
| `1487 × 1000` | 水平越界 0；四区布局、参数表单、Preview 与底部动作无裁切或碰撞 | 14 | 当前桌面主验收宽度；持久化 Ready 截图为同宽 `1487 × 1058` |
| `980 × 800` | 水平越界 0；命令索引按断点隐藏，主内容保持可读 | 9 | `≤980` 的既定响应式行为，不把隐藏的辅助索引算作缺失功能 |
| `680 × 700` | 水平越界 0；页面垂直堆叠，无水平裁切或控件碰撞 | 0（滚动顶端） | 低于 Tauri `minWidth=800` 的程序化压力测试；`≤680` 时全局导航隐藏，交互内容位于后续纵向区域，不能把顶端计数解释为完整键盘覆盖 |

三档均没有水平滚动条或核心内容横向截断。680 档仅证明 CSS 在宿主最小宽度以下仍不会横向破裂，不代表应用允许用户把真实 Tauri 窗口缩到 680。

## 当前键盘与可访问名称证据

- 在 1487 宽度的真实窗口中，从已选中的 PowerShell 命令开始连续 Tab。
- 实际焦点依次到达：CMD 命令、验证短等待、Text、Number、Boolean、Select、Folder 编辑、选择目录、添加目录和 annotation scroll group。
- 上述到达项均为 enabled、visible，并具有可访问名称；表单控件与按钮的 role/name 也由当前 91 项前端测试覆盖。
- Search 的程序化 `SetFocus` 本轮未成功，因此不声称本次真实键盘路径从搜索框起步；这项事实与已经成功覆盖的 Tab 路径分开记录。
- feature-only 验证短等待只在显式验证构建的键盘记录中出现；默认列表仍严格只有两个正式 Built-in。

## 当前真实宿主交互证据

- PowerShell 正式 Built-in：六类参数 Preview、64 个十六进制字符（256 bit）的 Hash、认证 UUID、六行精确 stdout、空 stderr、唯一 Finished 和 Exit Code 0 均通过；终态后 Run 消失并要求重新 Preview。
- CMD 正式 Built-in：包含多语言、Emoji、引号、反斜杠及 `% ! ^ & | < > ( )` 的 Text 原样输出；Preview 只显示内部绑定而不泄露 raw Text；空 stderr、唯一 Finished 和 Exit Code 0 均通过。
- feature-only Cancel：固定 5 秒 `Start-Sleep` 经同一 Workspace 完成 Preview → Run → Cancel；点击发生在 Run 后 92 ms，界面观察到 Starting/Running → Cancelling → 唯一 Cancelled，终态后没有残留匹配进程或 Active Artifact。
- 验收后已退出 feature 构建并恢复默认应用；短等待 Definition 不再可见。
- 两次正式运行前后两个输入目录的直属项数量和名称集合 SHA-256 完全一致。精确 Hash、UUID、stdout 顺序和目录指纹见 `docs/testing/测试与验收.md` 的 CMD02-H/S 清单。

## 控制台与运行时证据

### 本轮 CMD-02

- 尝试附加独立 WebView console，但 remote-debug 参数未生效，当前浏览器控制 runtime 也不可用，因此没有取得可独立复核的 fresh console error/warning/issue 计数。
- 不得把“未看到错误”写成“console=0”。本轮可以确认的范围是：PowerShell、CMD、Cancel、响应式和键盘流程没有出现可见错误提示或 Vite overlay；Vite/Tauri 会话没有编译或可见 runtime 错误；`pnpm check:web` 的 7 个测试文件、91 项测试、两套 TypeScript 检查和生产构建均通过。

### 历史视觉原型

- 原型阶段曾在 Chromium DevTools 完成独立检查：最终 reload 后 error、warning、issue 均为 0；Lighthouse Accessibility 100、Best Practices 100。
- 这份历史证据只证明当时的无副作用浏览器原型，不等价于本轮真实 Tauri/WebView fresh console 结果。

## 当前 Implementation Checklist

- [x] 默认双 Built-in 与六类真实 Definition 已替换原型假数据。
- [x] 当前 PowerShell Preview Ready 状态保存为 `1487 × 1058` 截图。
- [x] 单层 Tauri 标题栏及窗口控制保持通过。
- [x] PowerShell/CMD Preview、Run、精确 Output 与唯一 Finished 通过。
- [x] feature-only UI Cancel 与唯一 Cancelled 通过，默认构建已恢复为两条正式命令。
- [x] `1487`、`980`、`680` 三档无水平越界或核心控件碰撞。
- [x] 已记录一条真实可达的键盘 Tab 路径和所有到达项的可访问名称。
- [x] 不可信 Preview/Output 只作为文本显示的自动回归通过。
- [ ] 本轮独立 WebView console attachment 未取得；不以历史 Chromium console=0 替代。

## 原型历史（保留）

以下记录说明当前视觉语法如何从用户确认的 Hero Delete 原型收敛而来。它是设计历史，不是当前默认双 Built-in 的业务内容。

### 原型对照状态

- source visual truth path：`docs/design/Command-Workspace-视觉原型.png`
- source viewport / pixels：`1487 × 1058`
- 当时实现截图：同名 `docs/design/Command-Workspace-implementation-ready.png` 的前一历史版本，可从 Git 历史恢复；当前路径已经更新为真实 Rust Contract 的 PowerShell Preview Ready 状态。
- 当时 state：Hero Command 的 Ready 状态；3 个目标、Preview Ready、Safety Passed、破坏性动作可见，但永久删除按钮只打开无副作用说明。
- 当时 comparison input：把源图与实现截图合并为 `1487 × 2132` 的同宽临时对照输入；临时合并图未纳入仓库。

### 原型 Full-view / focused comparison

- 同尺寸对照确认了暖纸底、无卡片墙、细线分区、宋体标题、等宽证据、灰蓝稳定信号和砖红风险信号。
- 工作区右侧安全栏从标题区顶部贯穿到动作区；标题、Runner、目标、Preview、注释栏和底部动作线的主要边界对齐。
- 当时源图与实现图均为 `1487 × 1058`、device pixel ratio `1`，以同尺寸 1:1 对照；标题区底线约为实现 `y=242.6`、源图 `y=243`，Runner 底线约为 `y=337.6`，目标区底线约为 `y=534.6`，代码区约为 `y=663.6–867.1`，底部动作区从 `y=940` 开始。
- 标题与 Runner：对照标题占位、描述基线、`执行器 / 状态` 比例和标题区底线，没有换行或比例漂移。
- 目标与 Preview：对照三个 40px 目标行、Preview 摘要的上下声部、11 行带行号脚本和说明脚注；实现列没有纵向溢出，当时 `clientHeight` 与 `scrollHeight` 均为 `697`。
- 安全栏：对照顶部安全结论、弯箭头、身份核对、Runner 事实、执行前记录和修订记录；所有区段位于底部动作区之前。
- 导航与索引：对照六项全局导航、10 条可见原型 Command Block、品牌脚注和“共 24 个命令块”；窄轨道中的导航链接仍有明确可访问名称。
- 当时品牌标记同样复用 `src-tauri/icons/icon.png`，其他图标来自 Phosphor；没有文本符号、手绘 SVG、CSS 图形、占位图或拉伸图像。

### 原型 Comparison history

#### Pass 1 — blocked

- [P1] 安全栏从标题下方开始，改变了源图的主区域结构。
  - Fix：把 Command Workspace 改为两列三行网格；右栏跨越标题和证据两行，底部动作区跨越全宽。
- [P2] Runner 使用等分轨道，标题、目标区和 Preview 摘要的节奏与源图不同。
  - Fix：Runner 改为 `0.56fr / 1fr`；修正标题、目标和 Preview 的间距与上下声部。
- [P2] 索引只有 7 条可见命令，与源图 10 条可见项和总量 24 的密度不一致。
  - Fix：恢复 10 条现实原型摘要，并独立保留总量 24 的索引语境。
- [P2] 左栏品牌脚注、代码区和动作按钮的尺寸偏离源图。
  - Fix：重排品牌脚注与平台分隔线；给 Preview 增加行号并校准代码高度；按源图扩大两个动作按钮。
- Post-fix evidence：第二次 `1487 × 1058` 截图显示主区域边界、密度和底部动作线已经对齐；继续进入交互和可访问性检查。

#### Pass 2 — blocked

- [P2] `1240 × 900` 窄轨道下导航链接因可见文字隐藏而失去可访问名称；次要文字对比度约为 `4.38:1`。
  - Fix：为六个导航链接补充稳定 `aria-label`，把次要墨色调整为 `#676865`；搜索框补充 `id` 和 `name`，页面补充真实品牌 favicon。
- Post-fix evidence：重新加载后原型 Chromium console 没有 error、warning 或 issue；Lighthouse snapshot 为 Accessibility 100、Best Practices 100。

#### Pass 3 — passed

- 最终同尺寸合并对照中没有残留 P0、P1 或 P2。
- P3 字体差异分类为 Windows 系统字体回退的可接受变化。

### 原型浏览器与交互证据

- Chromium DevTools：完成搜索过滤、清空搜索、移除目标、恢复原型状态、添加文件夹说明、永久删除说明和 Escape 关闭对话框。
- 移除目标后：目标数量由 3 变为 2，状态变为“需要重新预览”，旧 Preview 被替换为失效说明，安全结论变为等待状态，永久删除按钮 disabled。
- 恢复后：三个目标和 Ready Preview 回归；点击永久删除只出现“前端视觉原型不会执行真实命令”的 modal。
- 响应式：检查 `1240 × 900`、`980 × 900` 和 Chromium 最小可用的 `500 × 812`；没有水平溢出或核心控件碰撞。`980` 时 `scrollWidth=965 <= innerWidth=980`，`500` 时 `scrollWidth=485 <= innerWidth=500`。
- 当时 Implementation Checklist：Ready 桌面状态同尺寸截图、源图与实现图同一比较输入、搜索/失效/恢复/禁用/说明对话框、桌面/窄桌面/小窗口、Chromium console 和 Lighthouse Accessibility 均完成。
- 当时 Open Questions：无。
- 原型最终结果：passed。

## Follow-up Polish

- 如未来决定内置字体资产，可再锁定不同 Windows 机器上的精确中文排版；当前不影响验收。
- 若后续需要把 fresh WebView console=0 作为发布门禁，应在可用的独立 WebView 调试连接中重新取得证据，不能沿用本轮限制或历史原型结论。

final result: current visual and interaction surfaces passed; fresh WebView console count not obtained and explicitly excluded from the claim
