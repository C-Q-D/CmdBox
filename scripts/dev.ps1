<#
启动 CmdBox 完整 Windows Tauri 增量开发环境。

默认在当前终端运行；使用 -Detached 会创建受控后台进程并等待 Vite 页面与真实 CmdBox 窗口就绪。
#>
[CmdletBinding()]
param(
    # 在后台运行完整 Tauri Dev，日志写入 .cmdbox。
    [switch]$Detached,

    # 后台模式等待 Vite 和原生窗口就绪的最长秒数。
    [ValidateRange(10, 600)]
    [int]$TimeoutSeconds = 180
)

$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "common.ps1")

$projectRoot = Get-CmdBoxProjectRoot
$runtimeDirectory = Get-CmdBoxRuntimeDirectory
$statePath = Join-Path $runtimeDirectory "tauri-dev.json"
$workerPath = Join-Path $PSScriptRoot "tauri-dev-worker.ps1"
$checkEnvironmentPath = Join-Path $PSScriptRoot "check-env.ps1"
$ensureDependenciesPath = Join-Path $PSScriptRoot "ensure-deps.ps1"
$stopScriptPath = Join-Path $PSScriptRoot "stop.ps1"

<# 等待后台进程同时提供 Vite 页面和真实、可见的 CmdBox 原生窗口。 #>
function Wait-TauriReady {
    param(
        # 后台 Tauri Dev 的稳定根进程。
        [System.Diagnostics.Process]$WorkerProcess
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($WorkerProcess.HasExited) {
            throw "Tauri Dev 在窗口就绪前退出，请检查 $runtimeDirectory 中的日志。"
        }

        $webReady = $false
        try {
            $response = Invoke-WebRequest `
                -Uri "http://localhost:1420" `
                -TimeoutSec 3 `
                -UseBasicParsing
            $webReady = $response.StatusCode -eq 200 -and $response.Content.Contains("CmdBox")
        }
        catch {
            $webReady = $false
        }

        $descendantIds = @(Get-CmdBoxDescendantProcessIds -RootProcessId $WorkerProcess.Id)
        $cmdBoxProcesses = @(
            Get-CimInstance Win32_Process -Filter "Name = 'cmdbox.exe'" |
                Where-Object { $descendantIds -contains [int]$_.ProcessId }
        )
        $windowReady = $false
        foreach ($cmdBoxProcess in $cmdBoxProcesses) {
            $nativeProcess = Get-Process -Id $cmdBoxProcess.ProcessId -ErrorAction SilentlyContinue
            if ($null -ne $nativeProcess -and $nativeProcess.MainWindowHandle -ne 0) {
                $windowReady = $true
                break
            }
        }

        if ($webReady -and $windowReady) {
            return
        }

        Start-Sleep -Seconds 1
    }

    throw "Tauri Dev 在 $TimeoutSeconds 秒内未同时得到 Vite 页面和 CmdBox 原生窗口。"
}

& $checkEnvironmentPath
& $ensureDependenciesPath

if (Test-Path -LiteralPath $statePath) {
    & $stopScriptPath
}

if (Test-CmdBoxDevPortInUse) {
    throw "端口 1420 已被占用；请先停止 Docker 前端或其他 Vite 实例。"
}

Set-Location -LiteralPath $projectRoot
$env:CARGO_INCREMENTAL = "1"

if (-not $Detached) {
    pnpm tauri dev
    if ($LASTEXITCODE -ne 0) {
        throw "Tauri Dev 异常退出，退出码：$LASTEXITCODE"
    }

    exit 0
}

New-Item -ItemType Directory -Path $runtimeDirectory -Force | Out-Null
$standardOutputPath = Join-Path $runtimeDirectory "tauri-dev.out.log"
$standardErrorPath = Join-Path $runtimeDirectory "tauri-dev.err.log"
$powerShellPath = (Get-Command pwsh -ErrorAction Stop).Source
$workerArguments = @(
    "-NoLogo",
    "-NoProfile",
    "-ExecutionPolicy", "Bypass",
    "-File", ('"{0}"' -f $workerPath)
)

$workerProcess = Start-Process `
    -FilePath $powerShellPath `
    -ArgumentList $workerArguments `
    -WorkingDirectory $projectRoot `
    -WindowStyle Hidden `
    -RedirectStandardOutput $standardOutputPath `
    -RedirectStandardError $standardErrorPath `
    -PassThru

[pscustomobject]@{
    # 用于精确停止整棵开发进程树的稳定根进程 ID。
    processId = $workerProcess.Id
    # 防止状态文件被复制到其他仓库后误用。
    projectRoot = $projectRoot
    # 停止前必须匹配的内部工作脚本路径。
    workerPath = $workerPath
    # 本地排障所需的启动时间。
    startedAt = [DateTimeOffset]::Now.ToString("O")
} | ConvertTo-Json | Set-Content -LiteralPath $statePath -Encoding utf8

try {
    Wait-TauriReady -WorkerProcess $workerProcess
}
catch {
    & $stopScriptPath
    throw
}

Write-Host "CmdBox Windows Tauri Dev 已在后台就绪。"
Write-Host "前端：http://localhost:1420"
Write-Host "日志：$standardOutputPath / $standardErrorPath"
Write-Host "停止命令：stop.cmd"
