<#
精确停止当前仓库的 Docker Compose Watch 与前端容器。

停止前会校验状态文件中的进程命令行确实属于当前 CmdBox 仓库，避免影响其他项目。
#>
[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$runtimeDirectory = Join-Path $projectRoot ".cmdbox"
$statePath = Join-Path $runtimeDirectory "web-watch.json"
$workerPath = Join-Path $PSScriptRoot "web-watch-worker.ps1"

<# 递归停止给定宿主进程及其子进程。 #>
function Stop-ProcessTree {
    param(
        # 已确认属于当前仓库的进程 ID。
        [int]$ProcessId
    )

    $children = Get-CimInstance Win32_Process -Filter "ParentProcessId = $ProcessId"
    foreach ($child in $children) {
        Stop-ProcessTree -ProcessId $child.ProcessId
    }

    Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue
}

if (Test-Path -LiteralPath $statePath) {
    $state = Get-Content -Raw -LiteralPath $statePath | ConvertFrom-Json
    $recordedProcessId = [int]$state.processId
    $process = Get-CimInstance Win32_Process -Filter "ProcessId = $recordedProcessId"

    if ($null -ne $process) {
        $belongsToWorker = $process.CommandLine.IndexOf(
            $workerPath,
            [StringComparison]::OrdinalIgnoreCase
        ) -ge 0
        $belongsToRepository = $process.CommandLine.IndexOf(
            $projectRoot,
            [StringComparison]::OrdinalIgnoreCase
        ) -ge 0

        if (-not ($belongsToWorker -and $belongsToRepository)) {
            throw "状态文件中的进程不属于当前仓库，已拒绝停止 PID $recordedProcessId。"
        }

        Stop-ProcessTree -ProcessId $recordedProcessId
    }

    Remove-Item -LiteralPath $statePath -Force
}

Set-Location -LiteralPath $projectRoot
$previousErrorActionPreference = $ErrorActionPreference
try {
    $ErrorActionPreference = "Continue"
    docker info --format "{{.ServerVersion}}" 2> $null | Out-Null
    $engineReady = $LASTEXITCODE -eq 0
}
finally {
    $ErrorActionPreference = $previousErrorActionPreference
}

if ($engineReady) {
    docker compose down --remove-orphans
    if ($LASTEXITCODE -ne 0) {
        throw "Docker Compose 前端容器清理失败，退出码：$LASTEXITCODE"
    }
}

Write-Host "CmdBox Docker 前端环境已停止。"
