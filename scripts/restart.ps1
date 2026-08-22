<# 停止并重新启动当前仓库的后台 Windows Tauri Dev。 #>
[CmdletBinding()]
param(
    # 等待重启后的 Vite 页面和原生窗口就绪的最长秒数。
    [ValidateRange(10, 600)]
    [int]$TimeoutSeconds = 180
)

$ErrorActionPreference = "Stop"
$stopScriptPath = Join-Path $PSScriptRoot "stop.ps1"
$devScriptPath = Join-Path $PSScriptRoot "dev.ps1"

& $stopScriptPath
& $devScriptPath -Detached -TimeoutSeconds $TimeoutSeconds

Write-Host "CmdBox Windows Tauri Dev 已完成重启。"
