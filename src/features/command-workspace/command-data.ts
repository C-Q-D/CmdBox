/** Command Workspace 当前显示的命令摘要。 */
export interface CommandSummary {
  /** Command Block 的稳定标识。 */
  id: string;
  /** 命令列表和工作区使用的名称。 */
  name: string;
  /** 搜索和摘要使用的简短说明。 */
  description: string;
  /** 当前固定展示的 Runner 摘要。 */
  runner: "PowerShell";
  /** 统一图标注册表中的图标标识。 */
  icon: "terminal" | "delete" | "calendar" | "rename" | "archive" | "hash" | "git";
  /** 当前是否为工作区选中命令。 */
  selected?: boolean;
}

/** 当前工作区索引可见的 Command Block 摘要。 */
export const commandSummaries: readonly CommandSummary[] = [
  {
    id: "execution-channel-diagnostic",
    name: "执行链路验收",
    description: "验证实时输出、自然结束与整树取消",
    runner: "PowerShell",
    icon: "terminal",
    selected: true,
  },
  {
    id: "delete-folders-permanently",
    name: "快速永久删除多个文件夹",
    description: "永久删除指定的多个文件夹",
    runner: "PowerShell",
    icon: "delete",
  },
  {
    id: "cleanup-old-temp-files",
    name: "清理大于 N 天的临时文件",
    description: "按最后修改时间清理临时目录",
    runner: "PowerShell",
    icon: "calendar",
  },
  {
    id: "batch-rename",
    name: "按扩展名批量重命名文件",
    description: "按命名规则统一文件名",
    runner: "PowerShell",
    icon: "rename",
  },
  {
    id: "archive-directory",
    name: "压缩并分卷备份目录",
    description: "创建可校验的本地归档",
    runner: "PowerShell",
    icon: "archive",
  },
  {
    id: "large-file-check",
    name: "查找重复文件（大小校验）",
    description: "按大小和散列定位重复项",
    runner: "PowerShell",
    icon: "hash",
  },
  {
    id: "checksum-manifest",
    name: "生成校验清单（SHA256）",
    description: "为交付目录生成文件校验清单",
    runner: "PowerShell",
    icon: "hash",
  },
  {
    id: "git-reset",
    name: "重置 Git 工作区（保留忽略）",
    description: "恢复工作区到可复现状态",
    runner: "PowerShell",
    icon: "git",
  },
  {
    id: "clear-build-cache",
    name: "清理构建缓存目录",
    description: "删除已知工具的构建缓存",
    runner: "PowerShell",
    icon: "delete",
  },
  {
    id: "registry-backup",
    name: "导出注册表备份",
    description: "导出指定注册表分支的本地备份",
    runner: "PowerShell",
    icon: "archive",
  },
];

/** 索引中显示的完整 Command Block 总量。 */
export const commandSummaryCount = 25;

/** 固定任务的用户可读范围，不包含可由前端改写的脚本文本。 */
export const fixedExecutionSteps = [
  "启动确定的 Windows PowerShell 5.1 非交互进程",
  "周期输出 stdout / stderr 与不可信文本样本",
  "创建受同一 Job 管理的诊断子进程",
  "自然结束时清理整个 Job 和临时脚本",
] as const;
