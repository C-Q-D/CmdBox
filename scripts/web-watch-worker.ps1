<#
CmdBox Docker Compose Watch 的内部前台进程。

该脚本只由 web-dev.ps1 的后台模式启动，使 Compose Watch 有稳定的宿主进程可供精确停止。
#>
[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

Set-Location -LiteralPath $projectRoot
docker compose up --build --watch --remove-orphans

if ($LASTEXITCODE -ne 0) {
    throw "Docker Compose Watch 异常退出，退出码：$LASTEXITCODE"
}
