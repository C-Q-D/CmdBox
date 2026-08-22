<#
检查 CmdBox Windows 主机开发所需的工具链。

检查过程只读，不安装系统工具、不修改代理配置。Docker Engine 默认只提示运行状态；传入
-RequireDocker 时会把 Engine 未运行视为失败。
#>
[CmdletBinding()]
param(
    # 要求 Docker Engine 当前必须可响应，用于 Docker 路径的严格验收。
    [switch]$RequireDocker
)

$ErrorActionPreference = "Stop"
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$results = New-Object System.Collections.ArrayList
$failures = New-Object System.Collections.ArrayList

<# 记录一个可读检查结果，并在失败时加入最终错误列表。 #>
function Add-EnvironmentResult {
    param(
        # 环境检查项名称。
        [string]$Name,

        # 通过、提示或失败。
        [string]$Status,

        # 检测到的版本、路径或处理建议。
        [string]$Detail,

        # 该检查失败时是否阻止完整 Tauri 开发。
        [bool]$Required = $true
    )

    [void]$results.Add([pscustomobject]@{
        # 用户可读的检查项名称。
        检查项 = $Name
        # 通过、提示或失败状态。
        状态 = $Status
        # 当前检测证据或下一步建议。
        详情 = $Detail
    })

    if ($Required -and $Status -eq "失败") {
        [void]$failures.Add("$Name：$Detail")
    }
}

<# 安全探测本地 HTTP 代理端口，不修改任何用户级包管理器配置。 #>
function Test-LocalProxy {
    $client = New-Object System.Net.Sockets.TcpClient
    try {
        $connection = $client.BeginConnect("127.0.0.1", 7897, $null, $null)
        if (-not $connection.AsyncWaitHandle.WaitOne(500)) {
            return $false
        }

        $client.EndConnect($connection)
        return $true
    }
    catch {
        return $false
    }
    finally {
        $client.Dispose()
    }
}

<# 探测 Docker Engine，并兼容 Windows PowerShell 5.1 的原生命令错误流。 #>
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

Add-EnvironmentResult `
    -Name "Windows" `
    -Status $(if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) { "通过" } else { "失败" }) `
    -Detail ([Environment]::OSVersion.VersionString)

$nodeCommand = Get-Command node -ErrorAction SilentlyContinue
if ($null -eq $nodeCommand) {
    Add-EnvironmentResult -Name "Node.js" -Status "失败" -Detail "未找到 node；需要 Node 24 LTS。"
}
else {
    $nodeVersionText = (& node --version).Trim().TrimStart("v")
    $nodeVersion = [Version]$nodeVersionText
    Add-EnvironmentResult `
        -Name "Node.js" `
        -Status $(if ($nodeVersion.Major -eq 24) { "通过" } else { "失败" }) `
        -Detail "$nodeVersionText（要求 24.x LTS）"
}

$pnpmCommand = Get-Command pnpm -ErrorAction SilentlyContinue
if ($null -eq $pnpmCommand) {
    Add-EnvironmentResult -Name "pnpm" -Status "失败" -Detail "未找到 pnpm；需要 11.x。"
}
else {
    $pnpmVersionText = (& pnpm --version).Trim()
    $pnpmVersion = [Version]$pnpmVersionText
    Add-EnvironmentResult `
        -Name "pnpm" `
        -Status $(if ($pnpmVersion.Major -eq 11) { "通过" } else { "失败" }) `
        -Detail "$pnpmVersionText（要求 11.x）"
}

$rustCommands = @("rustc", "cargo", "rustup")
foreach ($rustCommandName in $rustCommands) {
    if ($null -eq (Get-Command $rustCommandName -ErrorAction SilentlyContinue)) {
        Add-EnvironmentResult -Name $rustCommandName -Status "失败" -Detail "未找到 $rustCommandName。"
    }
}

if ($null -ne (Get-Command rustc -ErrorAction SilentlyContinue)) {
    $rustVersion = (& rustc --version).Trim()
    Add-EnvironmentResult -Name "Rust" -Status "通过" -Detail $rustVersion
}

if ($null -ne (Get-Command rustup -ErrorAction SilentlyContinue)) {
    $installedTargets = @(& rustup target list --installed)
    $msvcTargetReady = $installedTargets -contains "x86_64-pc-windows-msvc"
    Add-EnvironmentResult `
        -Name "Rust MSVC Target" `
        -Status $(if ($msvcTargetReady) { "通过" } else { "失败" }) `
        -Detail $(if ($msvcTargetReady) { "x86_64-pc-windows-msvc" } else { "缺少 x86_64-pc-windows-msvc" })
}

$vsWherePath = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path -LiteralPath $vsWherePath)) {
    Add-EnvironmentResult -Name "MSVC Build Tools" -Status "失败" -Detail "未找到 Visual Studio Installer 的 vswhere.exe。"
}
else {
    $installationPath = (& $vsWherePath `
        -latest `
        -products "*" `
        -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
        -property installationPath).Trim()

    if ([string]::IsNullOrWhiteSpace($installationPath)) {
        Add-EnvironmentResult -Name "MSVC Build Tools" -Status "失败" -Detail "未安装 C++ x64/x86 Build Tools 组件。"
    }
    else {
        $msvcToolsRoot = Join-Path $installationPath "VC\Tools\MSVC"
        $compiler = Get-ChildItem -LiteralPath $msvcToolsRoot -Directory -ErrorAction SilentlyContinue |
            Sort-Object Name -Descending |
            ForEach-Object { Join-Path $_.FullName "bin\Hostx64\x64\cl.exe" } |
            Where-Object { Test-Path -LiteralPath $_ } |
            Select-Object -First 1

        Add-EnvironmentResult `
            -Name "MSVC Build Tools" `
            -Status $(if ($null -ne $compiler) { "通过" } else { "失败" }) `
            -Detail $(if ($null -ne $compiler) { $compiler } else { "安装存在，但未找到 Hostx64/x64 cl.exe。" })
    }
}

$webViewRoot = Join-Path ${env:ProgramFiles(x86)} "Microsoft\EdgeWebView\Application"
$webViewVersion = Get-ChildItem -LiteralPath $webViewRoot -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match "^\d+(\.\d+)+$" } |
    Sort-Object { [Version]$_.Name } -Descending |
    Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName "msedgewebview2.exe") } |
    Select-Object -First 1

Add-EnvironmentResult `
    -Name "WebView2 Runtime" `
    -Status $(if ($null -ne $webViewVersion) { "通过" } else { "失败" }) `
    -Detail $(if ($null -ne $webViewVersion) { $webViewVersion.Name } else { "未找到 WebView2 Runtime。" })

$dockerCommand = Get-Command docker -ErrorAction SilentlyContinue
if ($null -eq $dockerCommand) {
    Add-EnvironmentResult -Name "Docker CLI" -Status "失败" -Detail "未找到 Docker CLI。"
}
else {
    $dockerVersion = (& docker --version).Trim()
    $composeVersion = (& docker compose version --short).Trim()
    Add-EnvironmentResult -Name "Docker / Compose" -Status "通过" -Detail "$dockerVersion；Compose $composeVersion"

    $dockerEngineReady = Test-DockerEngine
    if ($dockerEngineReady) {
        Add-EnvironmentResult -Name "Docker Engine" -Status "通过" -Detail "当前可用" -Required $RequireDocker
    }
    elseif ($RequireDocker) {
        Add-EnvironmentResult -Name "Docker Engine" -Status "失败" -Detail "当前未运行；web-dev.ps1 可自动启动。" -Required $true
    }
    else {
        Add-EnvironmentResult -Name "Docker Engine" -Status "提示" -Detail "当前未运行；web-dev.ps1 会按需自动启动。" -Required $false
    }
}

$proxyReady = Test-LocalProxy
Add-EnvironmentResult `
    -Name "本地包代理" `
    -Status $(if ($proxyReady) { "通过" } else { "提示" }) `
    -Detail $(if ($proxyReady) { "127.0.0.1:7897 可用；仅依赖安装进程使用。" } else { "127.0.0.1:7897 不可用；依赖安装将使用直连。" }) `
    -Required $false

Write-Host "CmdBox 开发环境检查（$projectRoot）"
$results | Format-Table -AutoSize

if ($failures.Count -gt 0) {
    throw "开发环境检查失败：`n- $($failures -join "`n- ")"
}

Write-Host "开发环境满足当前项目要求。"
