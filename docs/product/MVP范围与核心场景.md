# CmdBox MVP 范围与核心场景

> 来源：网页版 ChatGPT 对话“命令执行工具设计”（会话 ID：6a8970d8-9918-83e8-93f6-c194c8df47ea）。
> 保存方式：以下正文按当前会话读取接口返回内容原样保存，不对原讨论进行压缩或重写。

---

对，这个例子一下把产品的第一阶段定位拉得很清楚了。**我们现在甚至不需要把 CmdBox 做成“通用终端工具”，而应该优先做成“高频一次性系统命令的可视化执行器”。**

你的“删除超大文件夹”案例特别典型：真正麻烦的不是不知道 `Remove-Item` 或 `rmdir`，而是**每次都要重新找命令、替换路径、处理多个路径、担心引号和参数写错**。这意味着第一版最重要的能力不是 Terminal，而是 **Command Template + 参数表单 + 文件/文件夹选择器 + 安全执行**。

### 我会把 MVP 重新收缩成这个模型

一个 Command Block 的执行生命周期只有：

```text
选择命令块
   ↓
填写 / 选择参数
   ↓
生成最终命令
   ↓
确认
   ↓
启动子进程
   ↓
持续显示输出
   ↓
进程退出
   ↓
保存执行结果
```

也就是：

```text
Input → Run → stdout / stderr → Exit
```

明确**不做**：

```text
SSH 长连接
PowerShell 交互 Shell
CMD 交互 Shell
REPL
持续输入 stdin
Terminal Session
```

但是有一点要区分：**一次性命令不等于瞬间完成的命令。**

例如删一个几百万小文件的目录可能执行几分钟，所以仍然需要：

```text
运行中……
已运行 02:31

[终止任务]
```

只是任务结束之后进程就销毁，不进入交互模式。

---

你这个 Windows 删除案例还暴露出了一个我认为非常关键的产品需求：

## `Path List` 应该成为一级参数类型

之前我们设计的是：

```text
Text
Number
Password
Select
File
Folder
```

现在我认为第一版就应该增加：

```text
Multi File
Multi Folder
```

例如你的 PowerShell Command Block 不应该让用户自己写：

```powershell
@(
  'E:\xxx'
  'E:\yyy'
  'E:\zzz'
)
```

而应该直接做成：

```text
快速删除多个文件夹

要删除的文件夹

┌──────────────────────────────────────────┐
│ E:\comp\...\20260806-142314-b8a86c0d  × │
│ E:\comp\...\20260805-041753-80d7ee13  × │
│ E:\comp\...\20260806-003724-554407cd  × │
└──────────────────────────────────────────┘

[ + 添加文件夹 ]

删除方式
● PowerShell Remove-Item
○ CMD del + rmdir

                  [ 删除 3 个文件夹 ]
```

甚至支持直接从 Explorer：

```text
拖拽文件夹到 CmdBox
```

然后 CmdBox 自动把这三个目录转换成：

```powershell
@(
    'E:\folder1'
    'E:\folder2'
    'E:\folder3'
) | ForEach-Object {
    Remove-Item -LiteralPath $_ -Recurse -Force
}
```

用户根本不用处理 PowerShell 数组语法。

这其实就是产品价值所在。

---

## 而且我不建议让 Command Block 只有简单 `{{variable}}`

简单参数可以继续：

```powershell
Remove-Item -LiteralPath '{{folder}}' -Recurse -Force
```

但你的真实使用场景很快就会碰到：

```text
一个参数 = 多个值
```

因此模板系统应该至少支持：

```text
String
Number
Boolean
Path
Path[]
```

比如：

```yaml
name: 快速删除文件夹

shell: powershell

parameters:
  - key: paths
    label: 要删除的文件夹
    type: folder[]
    required: true
```

然后 Command Block 内部可以用一个简单的模板语义，比如：

```powershell
foreach ($path in {{paths:powershell-array}}) {
    Remove-Item -LiteralPath $path -Recurse -Force -ErrorAction Continue
}
```

系统负责把：

```text
E:\aaa
E:\bbb
E:\ccc
```

安全序列化成 PowerShell：

```powershell
@(
    'E:\aaa'
    'E:\bbb'
    'E:\ccc'
)
```

这里的**安全序列化非常重要**。

我们不能简单做字符串：

```text
{{path}} → 用户输入
```

因为路径里可能包含：

```text
空格
'
"
&
(
)
$
```

CmdBox 应该根据执行器自动做 escaping。

比如用户选择：

```text
D:\My Project\Test
```

应用自己负责生成合法 Shell 参数。

这会让 CmdBox 比“命令笔记 + 替换变量”高级一个层级。

---

## 你刚才那组笔记，其实可以直接变成几个 Command Block

我会拆成：

| Command Block | 输入参数 |
|---|---|
| 快速删除文件夹 | 一个或多个 Folder |
| 快速复制文件夹 | Source Folder、Target Folder、Threads |
| 快速移动文件夹 | Source、Target、模式 |
| PowerShell 批量删除 | 多个 Folder |
| Robocopy 高速复制 | Source、Target、Threads、Retry |
| Robocopy 跨盘移动 | Source、Target、Threads |

例如 **Robocopy 高速复制**：

```text
源目录
[D:\源文件夹          ] [选择]

目标目录
[E:\目标文件夹        ] [选择]

线程数
[32 ▼]

重试次数
[1 ]

等待时间
[1 秒]

☑ 不显示单文件进度

--------------------------------

命令预览

robocopy "D:\源文件夹" "E:\目标文件夹" /E /MT:32 /R:1 /W:1 /NP

                     [ ▶ 执行 ]
```

以前你的笔记是：

> 告诉未来的自己“这个命令怎么写”。

CmdBox 应该变成：

> 未来的自己完全不需要再关心这个命令怎么写。

这两个东西的产品体验差距非常大。

---

## “快速删除”可以直接作为 MVP 的 Hero Use Case

我甚至觉得产品第一版 Demo 就应该拿这个场景做。

首页可以有：

```text
常用命令

┌──────────────────────┐
│ 💥 快速删除文件夹     │
│ 跳过回收站直接删除    │
│                 运行 │
└──────────────────────┘

┌──────────────────────┐
│ 🚀 高速复制文件夹     │
│ Robocopy 多线程复制   │
│                 运行 │
└──────────────────────┘

┌──────────────────────┐
│ ⚡ 高速移动文件夹     │
│ 本地 / 跨盘快速移动   │
│                 运行 │
└──────────────────────┘
```

点“快速删除”以后：

```text
快速删除文件夹
Windows · CMD

选择要删除的文件夹

┌─────────────────────────────────────┐
│ D:\Projects\old-build             × │
│ D:\Projects\node_modules          × │
└─────────────────────────────────────┘

[+ 添加文件夹]

-------------------------------------

将永久删除 2 个文件夹
此操作不会进入回收站。

命令预览                         [复制]

del /f /s /q /a "..." && rmdir /s /q "..."

              [ 💥 永久删除 ]
```

执行之后：

```text
运行结果

✓ 执行完成

开始时间      18:04:21
耗时          47.3 秒
Exit Code     0

输出
────────────────────────────────

...

────────────────────────────────

[再次执行]
```

这个已经是一个完整且非常容易理解的产品故事。

---

## 删除命令还需要特殊的安全设计

像：

```powershell
Remove-Item -Recurse -Force
```

和：

```cmd
rmdir /s /q
```

属于**不可逆操作**。

所以 Command Block 最好带一个风险级别：

```text
Normal
Destructive
```

普通命令：

```text
[执行]
```

危险命令：

```text
[永久删除]
```

并在按钮附近明确显示：

> 文件将被永久删除，不经过回收站。

我不建议每次都弹一个非常烦的：

```text
确定吗？
真的确定吗？
```

否则又把效率做没了。

比较好的体验可能是第一次明显提示，之后只需要按钮本身有明确的 destructive semantics；如果目标异常，比如：

```text
C:\
C:\Windows
用户主目录
盘符根目录
```

再进行**强制二次确认或直接拦截**。

这属于必须从 MVP 就考虑的 guardrail。

---

## Command Block 还应该有一个新的属性：Runner

因为你贴的笔记已经出现了：

```text
PowerShell
CMD
Robocopy
```

所以一个 Block 不应该简单只有：

```text
shell: windows
```

而应该明确：

```text
Runner

PowerShell
CMD
Bash
Zsh
```

例如：

```yaml
name: Windows 快速删除文件夹

runner: cmd

command: |
  del /f /s /q /a "{{path}}\*.*" && rmdir /s /q "{{path}}"
```

另一个则是：

```yaml
name: PowerShell 批量删除

runner: powershell
```

这样 CmdBox 自己决定用什么 executable 去启动：

```text
cmd.exe

powershell.exe / pwsh.exe

bash

zsh
```

而不是让用户自己打开对应 Terminal。

---

## 所以现在 MVP 的产品边界已经可以砍得很干净了

**第一版要做：**

- 一次性命令
- CMD
- PowerShell
- Bash / Zsh
- Command Block
- 文本参数
- 数字参数
- 单文件 / 单文件夹
- 多文件 / 多文件夹
- Select
- Checkbox
- Working Directory
- Command Preview
- 一键执行
- stdout / stderr
- Exit Code
- 执行耗时
- 运行中终止
- 执行历史
- 搜索
- 分类 / 收藏
- 危险命令保护
- 参数安全 escaping

而这些全部可以先不碰：

```text
SSH
PTY
Terminal Emulator
Interactive stdin
Workflow
多步骤任务
云同步
团队
Marketplace
Plugin
Agent
定时任务
```

这一下开发量会小很多，而且产品价值并没有被削弱。

我现在对 CmdBox 第一版的定义也会相应调整成一句更加准确的话：

> **CmdBox 是一个把常用的一次性 CLI 命令封装成可参数化操作界面，并安全、快速重复执行的桌面工具。**

而你这个“几百万文件的大目录删除”场景，我建议直接作为 **MVP 第一个标准 Command Block + 产品首页 Demo 场景**。因为它非常准确地展示了为什么用户需要 CmdBox：不是“不会用 CLI”，而是**明明知道 CLI 更高效，却不想每次记忆、查找和重写 CLI。**
