<#
CmdBox 后台 Tauri Dev 的内部前台进程。

该脚本由 dev.ps1 -Detached 启动，作为 Vite、Cargo Watch 和 CmdBox 窗口进程树的稳定根节点。
#>
[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$env:CARGO_INCREMENTAL = "1"

if (Test-Path Variable:\PSNativeCommandUseErrorActionPreference) {
    $PSNativeCommandUseErrorActionPreference = $false
}

Set-Location -LiteralPath $projectRoot
pnpm tauri dev

if ($LASTEXITCODE -ne 0) {
    throw "Tauri Dev 异常退出，退出码：$LASTEXITCODE"
}
