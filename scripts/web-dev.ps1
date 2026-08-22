<#
启动 CmdBox 的 Docker 纯前端开发环境。

默认以前台 Compose Watch 运行，按 Ctrl+C 会进入清理流程；使用 -Detached 可在后台保持源码同步，
适合 AI 验证或需要继续使用当前终端的开发场景。
#>
[CmdletBinding()]
param(
    # 在后台运行 Compose Watch，并把日志写入仓库的 .cmdbox 目录。
    [switch]$Detached,

    # 等待 Docker Engine 和 Vite 健康的最长秒数。
    [ValidateRange(10, 600)]
    [int]$TimeoutSeconds = 180
)

$ErrorActionPreference = "Stop"
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$runtimeDirectory = Join-Path $projectRoot ".cmdbox"
$statePath = Join-Path $runtimeDirectory "web-watch.json"
$workerPath = Join-Path $PSScriptRoot "web-watch-worker.ps1"
$stopScriptPath = Join-Path $PSScriptRoot "web-stop.ps1"

<# 检查 Docker Desktop Engine 是否已经可以响应命令。 #>
function Test-DockerEngine {
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        docker info --format "{{.ServerVersion}}" 2> $null | Out-Null
        $engineReady = $LASTEXITCODE -eq 0
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }

    return $engineReady
}

<# 在需要时启动 Docker Desktop，并等待 Engine 可用。 #>
function Start-DockerEngine {
    if (Test-DockerEngine) {
        return
    }

    $desktopPath = Join-Path $env:ProgramFiles "Docker\Docker\Docker Desktop.exe"
    if (-not (Test-Path -LiteralPath $desktopPath)) {
        throw "Docker Desktop 未安装或不在默认位置：$desktopPath"
    }

    Write-Host "Docker Engine 尚未运行，正在启动 Docker Desktop..."
    Start-Process -FilePath $desktopPath -WindowStyle Hidden | Out-Null

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Seconds 2
        if (Test-DockerEngine) {
            Write-Host "Docker Engine 已就绪。"
            return
        }
    }

    throw "Docker Engine 在 $TimeoutSeconds 秒内未就绪，请打开 Docker Desktop 查看具体错误。"
}

<# 等待宿主固定端口返回 CmdBox 页面。 #>
function Wait-FrontendReady {
    param(
        # 后台 Watch 的宿主进程；提前退出时立即报告错误。
        [System.Diagnostics.Process]$WatchProcess
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($WatchProcess.HasExited) {
            throw "Docker Compose Watch 在前端就绪前退出，请检查 $runtimeDirectory 中的日志。"
        }

        try {
            $response = Invoke-WebRequest `
                -Uri "http://localhost:1420" `
                -TimeoutSec 3 `
                -UseBasicParsing
            if ($response.StatusCode -eq 200 -and $response.Content.Contains("CmdBox")) {
                return
            }
        }
        catch {
            # Vite 或容器仍在启动时继续等待，超时后统一报告。
        }

        Start-Sleep -Seconds 2
    }

    throw "CmdBox 前端在 $TimeoutSeconds 秒内未通过健康检查，请查看 Docker 与脚本日志。"
}

Set-Location -LiteralPath $projectRoot
Start-DockerEngine

if (-not $Detached) {
    try {
        docker compose up --build --watch --remove-orphans
        if ($LASTEXITCODE -ne 0) {
            throw "Docker Compose Watch 异常退出，退出码：$LASTEXITCODE"
        }
    }
    finally {
        docker compose down --remove-orphans
    }

    exit 0
}

if (Test-Path -LiteralPath $statePath) {
    & $stopScriptPath
}

New-Item -ItemType Directory -Path $runtimeDirectory -Force | Out-Null
$standardOutputPath = Join-Path $runtimeDirectory "web-watch.out.log"
$standardErrorPath = Join-Path $runtimeDirectory "web-watch.err.log"
$powerShellPath = (Get-Command pwsh -ErrorAction Stop).Source
$workerArguments = @(
    "-NoLogo",
    "-NoProfile",
    "-ExecutionPolicy", "Bypass",
    "-File", ('"{0}"' -f $workerPath)
)

$watchProcess = Start-Process `
    -FilePath $powerShellPath `
    -ArgumentList $workerArguments `
    -WorkingDirectory $projectRoot `
    -WindowStyle Hidden `
    -RedirectStandardOutput $standardOutputPath `
    -RedirectStandardError $standardErrorPath `
    -PassThru

[pscustomobject]@{
    processId = $watchProcess.Id
    projectRoot = $projectRoot
    workerPath = $workerPath
    startedAt = [DateTimeOffset]::Now.ToString("O")
} | ConvertTo-Json | Set-Content -LiteralPath $statePath -Encoding utf8

Wait-FrontendReady -WatchProcess $watchProcess
Write-Host "CmdBox 前端已在后台就绪：http://localhost:1420"
Write-Host "停止命令：powershell -ExecutionPolicy Bypass -File scripts/web-stop.ps1"
