<#
按项目依赖文件指纹安装 CmdBox 前端依赖。

只有 package.json、pnpm-lock.yaml 或 pnpm-workspace.yaml 内容变化，或 node_modules 不存在时，
才执行冻结锁文件安装。代理仅设置在当前脚本进程，不修改用户级 npm/pnpm 配置。
#>
[CmdletBinding()]
param(
    # 无视已有指纹，强制执行一次冻结锁文件安装。
    [switch]$Force
)

$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "common.ps1")

$projectRoot = Get-CmdBoxProjectRoot
$runtimeDirectory = Get-CmdBoxRuntimeDirectory
$statePath = Join-Path $runtimeDirectory "dependencies.json"
$dependencyFiles = @(
    (Join-Path $projectRoot "package.json"),
    (Join-Path $projectRoot "pnpm-lock.yaml"),
    (Join-Path $projectRoot "pnpm-workspace.yaml")
)

<# 计算所有依赖清单内容共同决定的稳定 SHA-256 指纹。 #>
function Get-DependencyFingerprint {
    $hashLines = @(
        $dependencyFiles | ForEach-Object {
            if (-not (Test-Path -LiteralPath $_)) {
                throw "缺少依赖文件：$_"
            }

            $fileBytes = [IO.File]::ReadAllBytes($_)
            $fileSha256 = [Security.Cryptography.SHA256]::Create()
            try {
                $fileHashBytes = $fileSha256.ComputeHash($fileBytes)
            }
            finally {
                $fileSha256.Dispose()
            }

            $fileHash = [BitConverter]::ToString($fileHashBytes).Replace("-", "")
            "{0}:{1}" -f ([IO.Path]::GetFileName($_)), $fileHash
        }
    )

    $bytes = [Text.Encoding]::UTF8.GetBytes($hashLines -join "`n")
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        $hash = $sha256.ComputeHash($bytes)
    }
    finally {
        $sha256.Dispose()
    }

    return ([BitConverter]::ToString($hash).Replace("-", "").ToLowerInvariant())
}

<# 安全探测用户约定的本地包代理端口。 #>
function Test-DependencyProxy {
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

$fingerprint = Get-DependencyFingerprint
$modulesReady = Test-Path -LiteralPath (Join-Path $projectRoot "node_modules\.pnpm")
$fingerprintMatches = $false

if (Test-Path -LiteralPath $statePath) {
    try {
        $state = Get-Content -Raw -LiteralPath $statePath | ConvertFrom-Json
        $fingerprintMatches = $state.fingerprint -eq $fingerprint
    }
    catch {
        $fingerprintMatches = $false
    }
}

if (-not $Force -and $modulesReady -and $fingerprintMatches) {
    Write-Host "依赖未变化，跳过 pnpm install。"
    return
}

if (Test-DependencyProxy) {
    $env:HTTP_PROXY = "http://127.0.0.1:7897"
    $env:HTTPS_PROXY = "http://127.0.0.1:7897"
    $env:NO_PROXY = "127.0.0.1,localhost"
    Write-Host "使用当前进程代理 127.0.0.1:7897 安装依赖。"
}
else {
    Write-Host "本地代理未运行，使用当前网络直连安装依赖。"
}

Set-Location -LiteralPath $projectRoot
pnpm install --frozen-lockfile --prefer-offline
if ($LASTEXITCODE -ne 0) {
    throw "pnpm install 失败，退出码：$LASTEXITCODE"
}

New-Item -ItemType Directory -Path $runtimeDirectory -Force | Out-Null
[pscustomobject]@{
    # 当前已安装依赖对应的清单内容指纹。
    fingerprint = $fingerprint
    # 便于排查本机依赖最后刷新时间。
    installedAt = [DateTimeOffset]::Now.ToString("O")
} | ConvertTo-Json | Set-Content -LiteralPath $statePath -Encoding utf8

Write-Host "依赖已按冻结锁文件准备完成。"
