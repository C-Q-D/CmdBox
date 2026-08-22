/** Command Workspace 视觉原型使用的命令摘要。 */
export interface PrototypeCommandSummary {
  id: string;
  name: string;
  description: string;
  runner: "PowerShell";
  icon: "delete" | "calendar" | "rename" | "archive" | "hash" | "git";
  selected?: boolean;
}

/** 视觉原型中可见的 Command Block 列表。 */
export const prototypeCommands: readonly PrototypeCommandSummary[] = [
  {
    id: "delete-folders-permanently",
    name: "快速永久删除多个文件夹",
    description: "永久删除指定的多个文件夹",
    runner: "PowerShell",
    icon: "delete",
    selected: true,
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
  {
    id: "delete-empty-directories",
    name: "删除空目录（递归）",
    description: "递归移除不包含文件的空目录",
    runner: "PowerShell",
    icon: "delete",
  },
];

/** 原型索引中显示的完整 Command Block 总量。 */
export const prototypeCommandCount = 24;

/** Rust Core 规范化结果的原型快照；不是前端自行计算的执行参数。 */
export const prototypeTargets = [
  String.raw`D:\项目缓存`,
  String.raw`E:\旧版构建产物`,
  String.raw`F:\临时 下载`,
] as const;

/** 只读 Preview 原型；真实产品中只能由 Rust Core 返回。 */
export const prototypePreview = `$ErrorActionPreference = 'Stop'
$targets = @(
  'D:\\项目缓存',
  'E:\\旧版构建产物',
  'F:\\临时 下载'
) | ForEach-Object { (Resolve-Path -LiteralPath $_).ProviderPath }

foreach ($p in $targets) {
  if (-not (Test-Path -LiteralPath $p)) { throw "路径不存在: $p" }
  Remove-Item -LiteralPath $p -Recurse -Force
}`;
