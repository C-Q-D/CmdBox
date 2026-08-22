<#
精确停止当前仓库由 dev.ps1 -Detached 启动的完整 Tauri 开发进程树。

状态文件中的根路径和工作脚本必须与当前仓库一致，否则拒绝终止任何进程。
#>
[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "common.ps1")

$projectRoot = Get-CmdBoxProjectRoot
$runtimeDirectory = Get-CmdBoxRuntimeDirectory
$statePath = Join-Path $runtimeDirectory "tauri-dev.json"
$workerPath = Join-Path $PSScriptRoot "tauri-dev-worker.ps1"

if (-not (Test-Path -LiteralPath $statePath)) {
    Write-Host "当前仓库没有已记录的后台 Tauri Dev。"
    return
}

$state = Get-Content -Raw -LiteralPath $statePath | ConvertFrom-Json
$recordedProcessId = [int]$state.processId

if (-not [string]::Equals($state.projectRoot, $projectRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "状态文件中的项目根目录与当前仓库不一致，已拒绝停止进程。"
}

$process = Get-CimInstance Win32_Process -Filter "ProcessId = $recordedProcessId"
if ($null -ne $process) {
    $belongsToWorker = Test-CmdBoxCommandLineContains `
        -CommandLine $process.CommandLine `
        -ExpectedText $workerPath
    $belongsToRepository = Test-CmdBoxCommandLineContains `
        -CommandLine $process.CommandLine `
        -ExpectedText $projectRoot

    if (-not ($belongsToWorker -and $belongsToRepository)) {
        throw "状态文件中的进程不属于当前仓库，已拒绝停止 PID $recordedProcessId。"
    }

    Stop-CmdBoxProcessTree -RootProcessId $recordedProcessId
}

$deadline = [DateTime]::UtcNow.AddSeconds(10)
while ([DateTime]::UtcNow -lt $deadline -and (Test-CmdBoxDevPortInUse)) {
    Start-Sleep -Milliseconds 250
}

$remainingCmdBoxProcesses = @(
    Get-CimInstance Win32_Process -Filter "Name = 'cmdbox.exe'" |
        Where-Object {
            -not [string]::IsNullOrWhiteSpace($_.ExecutablePath) -and
            $_.ExecutablePath.IndexOf($projectRoot, [StringComparison]::OrdinalIgnoreCase) -ge 0
        }
)

if ((Test-CmdBoxDevPortInUse) -or $remainingCmdBoxProcesses.Count -gt 0) {
    throw "后台根进程已停止，但仍检测到当前仓库的 Vite 端口或 CmdBox 进程，请检查日志。"
}

Remove-Item -LiteralPath $statePath -Force
Write-Host "CmdBox Windows Tauri Dev 已停止。"
