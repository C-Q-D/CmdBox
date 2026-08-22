# Command Workspace 视觉 QA

## 对照基线

- source visual truth path：`docs/design/Command-Workspace-视觉原型.png`
- implementation screenshot path：`docs/design/Command-Workspace-implementation-ready.png`
- viewport：`1487 × 1058` CSS px
- source pixels：`1487 × 1058`
- implementation pixels：`1487 × 1058`
- device pixel ratio：`1`
- density normalization：源图与实现图均为 1:1，同尺寸直接对照，无缩放或设备外框差异。
- state：Hero Command 的 Ready 状态；3 个目标、Preview Ready、Safety Passed、破坏性动作可见但只打开无副作用说明。
- comparison input：每轮都把上述两个持久化原图合并为 `1487 × 2132` 的同宽临时对照输入；临时合并图不纳入仓库，源图和最终实现图保留在上述路径。

## Findings

- 当前没有可执行的 P0、P1 或 P2 差异。
- [P3] 系统中文字体渲染存在轻微字面差异
  - Location：大标题、侧栏正文和小号等宽标签。
  - Evidence：源图是图像生成的印刷式中文排版；实现使用 `STSong`、`SimSun`、`Cascadia Mono` 等本机字体回退，层级、字重和占位接近，但不同 Windows 字体安装会产生轻微字宽与抗锯齿差异。
  - Impact：不改变布局、层级、可读性或操作，仅属于平台字体的细节变化。
  - Fix：当前无需修复；若未来引入受许可的内置字体资产，可再锁定跨机器字形。

## 必查表面

- Fonts and typography：大标题使用中文宋体声部，UI 正文使用系统无衬线，证据与技术事实使用等宽字体；标题、标签、正文、代码的层级和行高已逐区检查。没有截断核心信息，窄窗口标题仍完整显示。
- Spacing and layout rhythm：三栏框架、`188 + 296 + workspace` 外层轨道、`711 + 292` 工作区轨道、细规则线和底部动作区均与源图一致。实现的标题区底线约为 `y=242.6`，源图约为 `y=243`；Runner 底线约为 `y=337.6`，目标区底线约为 `y=534.6`，代码区约为 `y=663.6–867.1`，底部动作区从 `y=940` 开始。
- Colors and visual tokens：暖纸底、深墨、灰蓝证据、砖红风险、细灰规则线均由 CSS Token 统一控制；没有渐变和厚阴影。将次要墨色从 `#6f706c` 调整为 `#676865` 后，Lighthouse Accessibility 为 100。
- Image quality and asset fidelity：品牌标记直接复用 `src-tauri/icons/icon.png`，尺寸与透明背景正常；其余 UI 图标全部来自同一 Phosphor 线性图标家族，没有文本符号、手绘 SVG、CSS 图形、占位图或拉伸图像。源图没有其他照片或产品图像需要复刻。
- Copy and content：核心标题、三个目标、PowerShell、Preview、安全结论和不可恢复提示与确认内容一致。右侧修订事实采用当前 CmdBox 原型语义，不复制源图中没有项目依据的示例日期。

## Full-view comparison evidence

- 已将源图与最终实现截图合并到同一张 `1487 × 2132` 对照图中，以相同宽度、相同 Ready 状态逐区检查。
- 最终实现保持源图的编辑式研究手记语法：暖纸底、无卡片墙、细线分区、宋体标题、等宽证据、灰蓝稳定信号和砖红风险信号。
- 工作区右侧安全栏从标题区顶部贯穿到动作区；标题、Runner、目标、Preview、注释栏和底部动作线的主要边界已经对齐。

## Focused region comparison evidence

- 标题与 Runner：对照标题占位、描述基线、`执行器 / 状态` 比例和标题区底线；最终没有换行或比例漂移。
- 目标与 Preview：对照三个 40px 目标行、Preview 摘要的上下声部、11 行带行号脚本和说明脚注；实现列没有纵向溢出，`clientHeight` 与 `scrollHeight` 均为 `697`。
- 安全栏：对照顶部安全结论、弯箭头、身份核对、Runner 事实、执行前记录和修订记录；所有区段保持可见并落在底部动作区之前。
- 导航与索引：对照六项全局导航、10 条可见 Command Block、品牌脚注和“共 24 个命令块”；窄轨道中的导航链接仍有明确可访问名称。
- 这些区域在原始分辨率下均可读，因此无需另存裁剪图；对照依据仍是同一张合并输入，不是分离截图记忆判断。

## Comparison history

### Pass 1 — blocked

- [P1] 安全栏从标题下方开始，改变了源图的主区域结构。
  - Fix：把 Command Workspace 改为两列三行网格；右栏跨越标题和证据两行，底部动作区跨越全宽。
- [P2] Runner 使用等分轨道，标题、目标区和 Preview 摘要的节奏与源图不同。
  - Fix：Runner 改为 `0.56fr / 1fr`；修正标题、目标和 Preview 的间距与上下声部。
- [P2] 索引只有 7 条可见命令，与源图 10 条可见项和总量 24 的密度不一致。
  - Fix：恢复 10 条现实原型摘要，并独立保留总量 24 的索引语境。
- [P2] 左栏品牌脚注、代码区和动作按钮的尺寸偏离源图。
  - Fix：重排品牌脚注与平台分隔线；给 Preview 增加行号并校准代码高度；按源图扩大两个动作按钮。
- Post-fix evidence：第二次 `1487 × 1058` 截图显示主区域边界、密度和底部动作线已经对齐；继续进入交互和可访问性检查。

### Pass 2 — blocked

- [P2] `1240 × 900` 窄轨道下导航链接因可见文字隐藏而失去可访问名称；次要文字对比度约为 `4.38:1`。
  - Fix：为六个导航链接补充稳定 `aria-label`，把次要墨色调整为 `#676865`；搜索框补充 `id` 和 `name`，页面补充真实品牌 favicon。
- Post-fix evidence：重新加载后控制台没有 error、warning 或 issue；Lighthouse snapshot 为 Accessibility 100、Best Practices 100。

### Pass 3 — passed

- 最终同尺寸合并对照中没有残留 P0、P1 或 P2。
- P3 字体差异已明确分类为 Windows 系统字体回退的可接受变化。

## 浏览器与交互证据

- Chromium DevTools：完成搜索过滤、清空搜索、移除目标、恢复原型状态、添加文件夹说明、永久删除说明和 Escape 关闭对话框。
- 移除目标后：目标数量由 3 变为 2，状态变为“需要重新预览”，旧 Preview 被替换为失效说明，安全结论变为等待状态，永久删除按钮 disabled。
- 恢复后：三个目标和 Ready Preview 回归，永久删除按钮恢复；点击该按钮只出现“前端视觉原型不会执行真实命令”的 modal。
- 响应式：检查 `1240 × 900`、`980 × 900` 和 Chromium 最小可用的 `500 × 812`；没有水平溢出或核心控件碰撞。`980` 时 `scrollWidth=965 <= innerWidth=980`，`500` 时 `scrollWidth=485 <= innerWidth=500`。
- Console：最终 reload 后 error、warning、issue 均为 0。
- Lighthouse snapshot：Accessibility 100，Best Practices 100。SEO 与 Agentic Browsing 的 robots/llms 文件不属于本地 Windows 桌面视觉原型验收范围。

## Open Questions

- 无。

## Implementation Checklist

- [x] Ready 桌面状态同尺寸截图。
- [x] 源图与实现图放入同一比较输入。
- [x] 搜索、失效、恢复、禁用和说明对话框可操作。
- [x] 桌面、窄桌面和小窗口无水平溢出。
- [x] 控制台无错误、警告或浏览器 issue。
- [x] Lighthouse Accessibility 100。

## Follow-up Polish

- 如未来项目决定内置字体资产，可再锁定不同 Windows 机器上的精确中文排版；当前不影响验收。

final result: passed
