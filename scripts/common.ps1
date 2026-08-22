<# CmdBox Windows 开发脚本共享的路径与受控进程树能力。 #>

$script:CmdBoxProjectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

<# 返回当前 CmdBox 仓库的绝对路径。 #>
function Get-CmdBoxProjectRoot {
    return $script:CmdBoxProjectRoot
}

<# 返回被 Git 忽略的仓库运行状态目录。 #>
function Get-CmdBoxRuntimeDirectory {
    return Join-Path $script:CmdBoxProjectRoot ".cmdbox"
}

<# 判断命令行是否包含指定绝对路径，比较时忽略 Windows 大小写差异。 #>
function Test-CmdBoxCommandLineContains {
    param(
        # 待检查的完整进程命令行。
        [AllowNull()]
        [string]$CommandLine,

        # 必须出现在命令行中的绝对路径或稳定片段。
        [Parameter(Mandatory = $true)]
        [string]$ExpectedText
    )

    if ([string]::IsNullOrWhiteSpace($CommandLine)) {
        return $false
    }

    return $CommandLine.IndexOf(
        $ExpectedText,
        [StringComparison]::OrdinalIgnoreCase
    ) -ge 0
}

<# 返回给定根进程的全部后代进程 ID，不包含根进程自身。 #>
function Get-CmdBoxDescendantProcessIds {
    param(
        # 进程树根节点的进程 ID。
        [int]$RootProcessId
    )

    $processes = @(Get-CimInstance Win32_Process)
    $frontier = @($RootProcessId)
    $descendants = @()

    while ($frontier.Count -gt 0) {
        $children = @(
            $processes | Where-Object {
                $frontier -contains [int]$_.ParentProcessId
            }
        )

        if ($children.Count -eq 0) {
            break
        }

        $childIds = @($children | ForEach-Object { [int]$_.ProcessId })
        $descendants += $childIds
        $frontier = $childIds
    }

    return $descendants
}

<# 使用一次进程快照，从叶子节点开始停止已经确认属于当前仓库的进程树。 #>
function Stop-CmdBoxProcessTree {
    param(
        # 已完成归属校验的根进程 ID。
        [int]$RootProcessId
    )

    $descendantIds = @(Get-CmdBoxDescendantProcessIds -RootProcessId $RootProcessId)
    [Array]::Reverse($descendantIds)

    foreach ($descendantId in $descendantIds) {
        Stop-Process -Id $descendantId -Force -ErrorAction SilentlyContinue
    }

    Stop-Process -Id $RootProcessId -Force -ErrorAction SilentlyContinue
}

<# 判断 Vite 固定端口是否已有监听者。 #>
function Test-CmdBoxDevPortInUse {
    $client = New-Object System.Net.Sockets.TcpClient
    try {
        $connection = $client.BeginConnect("127.0.0.1", 1420, $null, $null)
        if (-not $connection.AsyncWaitHandle.WaitOne(250)) {
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
