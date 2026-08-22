# CmdBox 产品需求文档 PRD v0.1

> 来源：网页版 ChatGPT 对话“命令执行工具设计”（会话 ID：6a8970d8-9918-83e8-93f6-c194c8df47ea）。
> 保存方式：以下正文按当前会话读取接口返回内容原样保存，不对原讨论进行压缩或重写。

---

可以。先把它整理成一份 **PRD v0.1**，重点不是把所有功能写满，而是先把产品边界、核心对象和 MVP 定清楚，后面我们再逐项讨论。

:::writing{variant="document" id="48217" title="CmdBox 产品需求文档 PRD v0.1"}
# CmdBox 产品需求文档 PRD v0.1

## 1. 产品概述

### 1.1 产品名称

暂定名称：**CmdBox**

### 1.2 一句话定义

CmdBox 是一个将 Bash、PowerShell 等命令行操作封装成**可参数化、可搜索、可重复执行的命令块**的桌面工具。

用户不需要记住具体命令，只需要找到自己想完成的操作、填写必要参数，然后点击执行。

### 1.3 产品核心价值

传统 CLI 使用方式是：

**记忆命令 → 输入命令 → 修改参数 → 执行**

CmdBox 希望将其变成：

**找到任务 → 输入参数 → 点击执行**

核心理念：

> 用户应该记住“我要做什么”，而不是“这个命令怎么写”。

---

# 2. 用户痛点

目标用户日常存在大量重复性的 CLI 操作，例如：

- SSH 登录服务器
- 查看服务器状态
- Docker 容器管理
- Git 操作
- 文件压缩 / 解压
- ffmpeg 音视频处理
- 网络诊断
- kubectl 操作
- 数据库备份
- 项目启动
- 日志查询
- PowerShell 系统管理
- 自定义 Shell Script

目前常见解决方式存在明显问题。

### 2.1 依赖记忆

例如：

```bash
docker logs --tail 100 nginx-api
```

长期不用之后，很容易忘记：

- 参数叫什么
- 参数顺序
- flag 是什么
- 完整命令是什么

### 2.2 依赖 AI

忘记命令后需要：

1. 打开 AI
2. 描述需求
3. 获取命令
4. 修改参数
5. 复制
6. 打开终端
7. 执行

对于已经执行过很多次的操作，这个流程存在大量重复成本。

### 2.3 依赖笔记

另一种方式是保存到：

- Notion
- Obsidian
- Markdown
- txt
- README
- 收藏夹

但笔记解决的是：

> “把命令保存下来”

而没有解决：

> “快速找到并执行命令”

仍然需要搜索、复制、修改参数、切换 Terminal。

### 2.4 参数容易出错

很多命令只是部分参数发生变化，例如：

```bash
ssh root@192.168.1.100 -p 22
```

实际变化的可能只有：

- username
- host
- port

如果每次重新编辑完整命令，容易出现误删、拼写错误、参数错误。

---

# 3. 产品目标

CmdBox 的目标不是替代 Terminal。

CmdBox 要解决的是：

> **高频、重复、参数化 CLI 操作的管理与执行。**

产品最终希望成为用户自己的：

**Command Control Center**

所有常用 CLI 操作都可以被封装、搜索和重复执行。

---

# 4. 核心产品对象

CmdBox 最核心的数据对象称为：

## Command Block

一个 Command Block 代表一个可以被用户执行的任务。

例如：

**SSH 登录服务器**

其底层命令：

```bash
ssh {{username}}@{{host}} -p {{port}}
```

用户看到的不是一整条复杂命令，而是一张操作表单：

用户名：

```text
root
```

服务器地址：

```text
192.168.1.100
```

端口：

```text
22
```

然后点击：

**执行命令**

CmdBox 自动生成：

```bash
ssh root@192.168.1.100 -p 22
```

并执行。

---

# 5. Command Block 数据模型

初步建议一个 Command Block 至少包含以下信息。

```yaml
id: ssh-server

name: SSH 登录服务器

description: SSH 连接指定服务器

category: 服务器管理

shell: bash

command: |
  ssh {{username}}@{{host}} -p {{port}}

parameters:
  - key: username
    label: 用户名
    type: text
    default: root
    required: true

  - key: host
    label: 服务器地址
    type: text
    required: true

  - key: port
    label: 端口
    type: number
    default: 22

working_directory: null

environment: {}

tags:
  - ssh
  - server
```

---

# 6. 参数系统

参数系统是 CmdBox 与普通 Snippet Manager 最重要的区别之一。

MVP 建议支持以下参数类型。

| 类型 | 用途 |
|---|---|
| Text | 普通字符串 |
| Number | 数字 |
| Password | 密码 / Token |
| Select | 固定选项 |
| Checkbox | Boolean 参数 |
| File | 文件路径 |
| Folder | 文件夹路径 |
| Multiline | 多行文本 |

例如：

```bash
git checkout {{branch}}
```

系统识别到：

```text
{{branch}}
```

后生成：

```text
Branch
[ main ]
```

---

# 7. 产品核心流程

## 7.1 创建命令块

用户点击：

**新建命令块**

填写：

- 名称
- 描述
- 分类
- Shell 类型
- Command
- 参数
- 工作目录
- 环境变量

例如：

```bash
docker logs --tail {{lines}} {{container}}
```

系统检测：

```text
lines
container
```

并自动创建两个参数。

用户可以进一步设置：

```text
lines
类型：Number
默认值：100

container
类型：Text
```

保存 Command Block。

---

## 7.2 执行命令块

用户通过：

- 分类
- 搜索
- 最近使用
- 收藏

找到 Command Block。

点击后进入执行界面。

例如：

### Docker 查看日志

容器：

```text
nginx-api
```

日志条数：

```text
100
```

命令预览：

```bash
docker logs --tail 100 nginx-api
```

点击：

**执行命令**

---

## 7.3 查看执行结果

系统展示：

```text
$ docker logs --tail 100 nginx-api
```

下面展示 stdout / stderr：

```text
2026-08-22 10:24:51 GET /api/users
2026-08-22 10:24:52 GET /api/orders
```

同时记录：

- Exit Code
- 执行时间
- 开始时间
- 工作目录
- Shell
- 实际执行命令

例如：

```text
Exit Code: 0
Duration: 1.23s
Shell: /bin/zsh
Working Directory: ~/Projects/project-a
```

---

# 8. 信息架构

初步采用桌面应用布局。

## 8.1 左侧导航

一级导航：

```text
全部命令
最近使用
收藏夹

分类
├─ 开发工具
├─ 服务器管理
├─ Docker
├─ Git
├─ 网络工具
└─ 日常脚本

设置
```

支持显示数量：

```text
全部命令       24
最近使用        8
收藏夹          3
Docker          4
Git             3
```

---

# 9. 主工作区

建议采用三栏结构。

## 第一栏：Command List

当前分类：

```text
服务器管理
```

Command Blocks：

```text
SSH 登录服务器
查看服务器状态
重启服务器
查看日志
备份数据库
```

支持：

- 搜索
- 收藏
- 排序
- 新建
- 删除
- Duplicate

---

## 第二栏：Command Runner

展示当前 Command Block。

例如：

# SSH 登录服务器

```text
Bash
```

参数配置：

```text
用户名
[root]

服务器地址
[192.168.1.100]

端口
[22]

密码
[••••••••]
```

高级选项：

```text
Working Directory
Environment Variables
Timeout
Shell
```

命令预览：

```bash
ssh root@192.168.1.100 -p 22
```

主要 CTA：

**执行命令**

---

## 第三栏：Execution Result

执行状态：

```text
✓ 命令执行成功

耗时 1.23s
Exit Code 0
```

Terminal Output：

```text
$ ssh root@192.168.1.100 -p 22

Welcome to Ubuntu 24.04 LTS
root@server:~#
```

底部可以提供：

```text
输出
详细信息
```

---

# 10. 搜索系统

搜索应该成为 CmdBox 的核心交互之一。

支持搜索：

- Command 名称
- Description
- Tag
- Category
- Command 本身

例如用户输入：

```text
docker log
```

即可找到：

```text
Docker 查看日志
```

未来可以考虑支持 Command Palette：

```text
⌘ K
```

直接输入：

```text
ssh
```

找到：

```text
SSH 登录服务器
```

回车即可打开。

---

# 11. Shell 支持

MVP 建议至少支持：

## macOS / Linux

- Bash
- Zsh

## Windows

- PowerShell
- CMD

未来考虑：

- Fish
- WSL
- Python
- Node.js
- SSH Remote
- Docker Exec

---

# 12. 工作目录

Command Block 可以设置 Working Directory。

例如：

```text
~/Projects/project-a
```

执行：

```bash
npm run build
```

实际效果相当于：

```bash
cd ~/Projects/project-a
npm run build
```

Working Directory 可以：

- 固定
- 每次选择
- 使用变量

例如：

```text
{{project_directory}}
```

---

# 13. 环境变量

Command Block 可以设置：

```text
NODE_ENV=production
API_URL=https://api.example.com
```

实际执行：

```bash
NODE_ENV=production npm run build
```

敏感变量，例如：

```text
API_KEY
PASSWORD
TOKEN
```

需要作为 Secret 保存。

---

# 14. 安全机制

这是 CmdBox 非常重要的一部分。

因为产品本质上拥有：

> **执行本地命令的能力**

因此必须从产品初期设计安全边界。

## 14.1 命令预览

默认情况下执行前展示完整 Command。

例如：

```bash
rm -rf ./dist
```

用户明确知道即将执行什么。

## 14.2 危险命令提示

未来可以对危险命令进行检测，例如：

```bash
rm -rf
sudo
diskpart
format
shutdown
reboot
Remove-Item -Recurse
```

显示额外确认。

## 14.3 Secret 不进入日志

例如：

```text
password
token
api_key
```

Command Preview 中显示：

```text
********
```

执行历史中同样进行脱敏。

## 14.4 本地优先

第一阶段建议：

Command Block、执行历史、Secret 等默认保存在本地。

尽量避免上传：

- Command
- 文件路径
- Secret
- Terminal Output

---

# 15. 执行历史

每次运行记录：

```text
Command Block
Command
参数
执行时间
Duration
Exit Code
Result
Working Directory
```

例如：

```text
SSH 登录服务器

ssh root@192.168.1.100 -p 22

成功
1.23s
2026-08-22 14:32
```

支持：

**再次执行**

这样用户无需重新输入参数。

---

# 16. MVP 功能范围

第一版目标不是制作完整自动化平台。

第一版需要验证的问题只有一个：

> 用户是否愿意把常用命令放进 CmdBox，并通过 Command Block 重复执行？

因此 MVP 建议只实现以下核心能力。

### P0

- 创建 Command Block
- 编辑 Command Block
- 删除 Command Block
- Bash / Zsh / PowerShell 支持
- `{{variable}}` 参数模板
- 参数表单
- Command Preview
- 本地执行命令
- stdout / stderr 展示
- Exit Code
- Command 搜索
- Category 分类
- 收藏
- 最近使用
- 执行历史
- Working Directory
- Environment Variables

### P1

- Select 参数
- File Picker
- Folder Picker
- Password / Secret
- Command Duplicate
- Import / Export
- Keyboard Shortcut
- Command Palette
- Dangerous Command Warning

### P2

- AI 创建 Command Block
- 多步骤 Workflow
- SSH Remote Execution
- Docker Execution Environment
- Variables Library
- Team Sharing
- Cloud Sync
- Marketplace

---

# 17. AI 能力

AI 不应该成为第一版 CmdBox 的核心执行方式。

AI 最适合解决的是：

> **Command Block 的创建成本。**

例如用户输入：

```text
帮我做一个命令，可以查看指定 Docker 容器最后 N 条日志。
```

AI 自动生成：

```bash
docker logs --tail {{lines}} {{container}}
```

并识别：

```text
container
类型：Text

lines
类型：Number
默认：100
```

然后用户点击：

**保存为 Command Block**

形成：

```text
不知道命令
    ↓
询问 AI
    ↓
生成 Command Block
    ↓
保存
    ↓
以后重复执行
```

而不是：

```text
每一次执行
    ↓
重新询问 AI
```

---

# 18. Workflow

第一阶段暂不作为 MVP 核心能力。

未来一个 Workflow 可以包含多个 Step。

例如：

# 部署项目

Step 1：

```bash
git checkout {{branch}}
```

Step 2：

```bash
git pull origin {{branch}}
```

Step 3：

```bash
docker compose build
```

Step 4：

```bash
docker compose up -d
```

系统按照顺序执行。

可以支持：

```text
成功 → 下一步
失败 → 停止
```

最终 CmdBox 可以从：

**Command Runner**

逐渐升级为：

**Local Automation Platform**

---

# 19. 与其他产品的区别

## Terminal

Terminal 提供命令执行能力。

CmdBox 提供：

```text
命令管理
+
参数表单
+
任务抽象
+
重复执行
```

CmdBox 不替代 Terminal。

---

## Snippet Manager

Snippet Manager：

```text
保存 → 搜索 → 复制
```

CmdBox：

```text
保存 → 参数化 → 执行 → 查看结果
```

这是两者最核心的差异。

---

## Raycast

Raycast 更接近：

```text
Command Launcher
```

CmdBox 更强调：

```text
Parameterized Command
+
Execution UI
+
Execution History
```

---

## Postman

Postman 将：

```text
HTTP Request
```

封装成可保存、配置、重复执行的对象。

CmdBox 做的是类似的事情，只是核心对象变成：

```text
Shell Command
```

可以理解为：

> **Postman for CLI Commands**

---

# 20. 产品设计原则

## 原则一：默认简单

一个最简单的 Command Block 应该只需要：

```text
名称
Command
```

即可运行。

复杂参数属于高级能力。

## 原则二：执行前透明

用户应该永远可以看到：

> 即将执行什么命令。

CmdBox 不应该成为黑盒。

## 原则三：减少重复输入

用户输入过的内容应该尽可能：

- 保存
- 推荐
- 自动填充

## 原则四：Local First

用户的：

```text
命令
路径
服务器
Token
执行记录
```

都具有一定隐私属性。

默认应尽量在本地完成。

## 原则五：键盘友好

CLI 用户通常是 Keyboard-heavy 用户。

长期应该支持：

```text
⌘ K
搜索

↑ ↓
选择

Enter
打开

⌘ Enter
执行
```

---

# 21. 第一阶段用户画像

第一阶段不建议定位普通大众用户。

核心用户建议为：

### Developer

经常使用：

```text
Git
Docker
npm
pnpm
SSH
ffmpeg
curl
kubectl
```

### DevOps / SRE

经常使用：

```text
SSH
Docker
Kubernetes
Logs
Database
Server Management
```

### Power User

有大量：

```text
Bash Script
PowerShell
ffmpeg
文件处理
网络工具
自动化脚本
```

这类用户已经具备 CLI 使用基础，但希望减少记忆和重复输入成本。

---

# 22. MVP 成功指标

产品第一阶段可以重点观察：

## Command 创建数量

例如：

```text
平均每个活跃用户创建 ≥ 10 个 Command Blocks
```

说明用户愿意把自己的命令库迁移进产品。

## 重复执行率

同一 Command Block 是否被执行多次。

这是最关键指标之一。

如果大量 Command：

```text
Create Once
Execute Once
```

说明产品没有形成持续价值。

理想状态应该是：

```text
Create Once
Execute Many Times
```

## 搜索 → 执行时间

目标：

用户找到并执行一个已有命令：

```text
< 10 秒
```

## 用户留存

核心判断：

> 用户第二周是否仍然会回来执行自己的 Command Blocks。

---

# 23. 暂不考虑的能力

为了避免第一版失控，以下能力暂时不进入 MVP：

- 完整 Terminal Emulator
- IDE
- Shell Script IDE
- CI/CD 平台
- 复杂 No-Code Workflow
- 团队权限系统
- Marketplace
- Plugin System
- Cloud Runner
- Server Agent
- Mobile App

---

# 24. 产品发展路径

建议产品演进顺序：

### Stage 1

**Command Manager**

解决：

```text
保存
搜索
参数化
执行
```

### Stage 2

**Command Workflow**

解决：

```text
多个 Command 组合
```

### Stage 3

**AI Command Builder**

解决：

```text
自然语言 → Command Block
```

### Stage 4

**Automation Platform**

解决：

```text
Workflow
Remote
Schedule
Trigger
Team
```

---

# 25. 当前需要重点讨论的问题

在进入 UI 和技术实现之前，建议优先确定以下产品决策。

### Q1. Command Block 是否默认“一条命令”？

例如：

```bash
docker logs {{container}}
```

还是允许：

```bash
cd {{directory}}
git pull
npm install
npm run build
```

即第一版是否直接支持多行脚本。

### Q2. 执行模式是什么？

两种方向：

**模式 A：一次性执行**

```text
Command → stdout → 结束
```

适用于：

```text
Git
Docker
ffmpeg
脚本
```

**模式 B：Interactive Session**

例如：

```bash
ssh root@server
```

进入长期交互式 Terminal。

SSH、mysql、python REPL 等都属于这种情况。

这两种模式技术实现和 UI 复杂度差异很大。

### Q3. 是否把 Terminal 内嵌进 CmdBox？

如果支持 SSH 等 Interactive Command，就需要考虑：

```text
PTY
Terminal Emulator
Interactive stdin
Ctrl+C
Resize
ANSI
```

### Q4. 第一版平台

需要确定：

```text
macOS only
```

还是：

```text
macOS + Windows
```

因为 Bash 和 PowerShell 的环境差异较大。

### Q5. Command 配置采用 UI 还是源码模式？

一种是：

```text
Form Builder
```

另一种是：

```yaml
Command YAML
```

也可以两者同时存在：

```text
普通用户 → UI

高级用户 → YAML / JSON
```

### Q6. 参数是否保存上次输入值？

例如用户第一次：

```text
host = 192.168.1.100
username = root
```

第二次进入是否默认自动带出来。

倾向：**是。**

因为这会显著减少重复输入。

### Q7. Secret 如何保存？

需要决定：

```text
系统 Keychain
Windows Credential Manager
```

而不是直接保存在普通数据库。

---

# 26. 当前产品核心闭环

第一阶段必须把这个闭环做到足够顺滑：

```text
创建 Command Block
        ↓
定义参数
        ↓
保存
        ↓
搜索 / 分类找到
        ↓
填写参数
        ↓
预览 Command
        ↓
执行
        ↓
查看结果
        ↓
保存执行历史
        ↓
下一次快速再次执行
```

如果这个闭环体验足够好，CmdBox 就已经具有明确的独立产品价值。
:::

我建议下一步先别继续堆功能，而是把文档里的几个关键分叉定掉，尤其是 **“一次性命令 vs 交互式命令”**。这个决定会直接影响整个产品架构；例如你原型里的 SSH 登录，其实已经属于交互式 Terminal，而不只是普通的 `执行 → 输出 → 结束`。
