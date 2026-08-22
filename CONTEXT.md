# CmdBox 领域语言

CmdBox 领域围绕可重复的一次性命令、用户确认的执行预览和可验证的执行结果展开。本文件只统一项目术语，不保存实现方案。

## Language

**Command Block**：
一个可重复使用的命令定义，组合任务含义、Runner、类型化参数、模板、安全策略和结果策略；它不是某次实际运行。
_Avoid_：命令页面、脚本卡片、Snippet

**Parameter Definition**：
Command Block 对一个业务输入的类型、名称、约束、默认值和记忆策略的声明。
_Avoid_：表单字段、UI 配置

**Parameter Value**：
用户为一次 Preview 或 Execution 提交的结构化业务值，必须符合对应 Parameter Definition。
_Avoid_：命令字符串、Shell 参数

**Parameterless Command**：
Parameter Definition 集合为空的 Command Block；它不需要用户填写参数，但仍必须经过 Preview 才能执行。
_Avoid_：无界面命令、直接执行命令

**Command Workspace**：
用户配置、预览、执行一个当前 Command Block，并观察当前 Execution 的统一工作区；不同命令不会拥有各自独立页面。
_Avoid_：命令页面、自定义命令界面

**Preview**：
Rust Core 根据当前 Command Block 与 Parameter Value 生成的规范化摘要、安全结论和可读执行内容，并绑定完整 Execution Spec Hash。
_Avoid_：代码片段、前端预览、确认文本

**Execution**：
由一次已确认 Preview 启动的独立运行实例，拥有自己的标识、生命周期、输出和最终结果。
_Avoid_：Command Block、进程、任务页面

**Lifecycle**：
Execution 从准备、启动、运行、取消到终止的事实状态，不表达命令业务上是否成功。
_Avoid_：结果、Outcome

**Outcome**：
依据 Command Block 的 Outcome Policy 对 Execution 业务结果作出的解释，可为成功、警告、部分失败、失败或无结论。
_Avoid_：Lifecycle、Exit Code

**Safety Decision**：
Rust Core 在 Preview 或 Run 阶段对当前规范化目标和 Safety Policy 作出的结构化安全结论。
_Avoid_：前端警告、确认弹窗
