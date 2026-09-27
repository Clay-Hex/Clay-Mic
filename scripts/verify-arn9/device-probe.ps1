# device-probe.ps1 — ARN9 / 小米遥控器设备定位探针（第 1 步，go/no-go）
#
# 目的：确认遥控器是否以 HID-over-GATT + WUDFHost 形式注册，
#       以及能否拿到 WUDFDiagnosticInfo\HostPid（Frida 注入的目标 PID）。
#
# 用法（普通 PowerShell 即可，读 HKLM 不需要管理员）：
#   powershell -ExecutionPolicy Bypass -File scripts\verify-arn9\device-probe.ps1
#
# 通过标准（GO）：
#   1) 找到遥控器条目，VidPid 形如 dev_vid&XXXXXXXX_pid&XXXXXXXX
#   2) 该实例下存在 Device Parameters\WUDFDiagnosticInfo，HostPid > 0
#   3) 对应进程名为 wudfhost.exe
# 不通过（NO-GO）：没有任何 BTHLEDevice 条目带有效 HostPid
#   → 该遥控器不走 WUDF HID-over-GATT，Frida-WUDFHost 方案不适用，停止并回报。

$ErrorActionPreference = 'SilentlyContinue'

function Write-Section($title) {
    Write-Host ''
    Write-Host ("=== " + $title + " ===") -ForegroundColor Cyan
}

# ---------------------------------------------------------------------------
# 1) 蓝牙/GATT 服务设备枚举
# ---------------------------------------------------------------------------
Write-Section 'Bluetooth / BLE devices (Get-PnpDevice)'

Get-PnpDevice -Class Bluetooth | Sort-Object FriendlyName |
    Select-Object Status, FriendlyName, InstanceId |
    Format-Table -AutoSize | Out-String -Width 300 | Write-Host

# 也扫一下 HIDClass（有的遥控器枚举成 HID 设备）
Write-Section 'HIDClass devices'
Get-PnpDevice -Class HIDClass -ErrorAction SilentlyContinue | Sort-Object FriendlyName |
    Select-Object Status, FriendlyName, InstanceId |
    Format-Table -AutoSize | Out-String -Width 300 | Write-Host

# ---------------------------------------------------------------------------
# 2) BTHLEDevice 注册表树：找 WUDFDiagnosticInfo\HostPid
# ---------------------------------------------------------------------------
Write-Section 'HKLM\SYSTEM\CurrentControlSet\Enum\BTHLEDevice  (WUDF HostPid)'

$base = 'HKLM:\SYSTEM\CurrentControlSet\Enum\BTHLEDevice'
$found = $false

if (-not (Test-Path $base)) {
    Write-Host 'NO-GO: BTHLEDevice key not found.' -ForegroundColor Red
} else {
    # 实际层级是 2 层：BTHLEDevice\{GUID}_VID&…\实例ID\Device Parameters\WUDFDiagnosticInfo
    # 不硬编码深度，直接递归找 WUDFDiagnosticInfo，避免层级差异误判。
    $diagKeys = Get-ChildItem $base -Recurse -ErrorAction SilentlyContinue |
        Where-Object { $_.PSChildName -eq 'WUDFDiagnosticInfo' }

    foreach ($diagKey in $diagKeys) {
        $hostPid = (Get-ItemProperty -Path $diagKey.PSPath -Name HostPid -ErrorAction SilentlyContinue).HostPid

        # 回溯设备实例键：…\{GUID}_VID&…\实例ID\Device Parameters\WUDFDiagnosticInfo
        $deviceInstanceKey = $diagKey.Parent.Parent   # 实例ID
        $deviceKey = $deviceInstanceKey.Parent        # {GUID}_VID&…（含 VID/PID）

        $procName = $null
        $procIdOk = $false
        if ($hostPid -and $hostPid -gt 0) {
            $proc = Get-Process -Id $hostPid -ErrorAction SilentlyContinue
            if ($proc) {
                $procName = $proc.ProcessName
                $procIdOk = ($procName -ieq 'wudfhost')
            }
        }

        $isCandidate = $deviceKey.PSChildName -match 'vid&|pid&|2717|32b8|remote|mi|xiaomi|arn' `
            -or $deviceKey.PSChildName -match '1812'
        $mark = if ($procIdOk) { 'GO ' } elseif ($hostPid) { 'PID? ' } else { 'NO  ' }
        $color = if ($procIdOk) { 'Green' } elseif ($hostPid) { 'Yellow' } else { 'Gray' }

        $line = ('{0} Device={1}  Instance={2}  HostPid={3}  Proc={4}' -f `
            $mark, $deviceKey.PSChildName, $deviceInstanceKey.PSChildName,
            $(if ($hostPid) { $hostPid } else { '-' }),
            $(if ($procName) { $procName } else { '-' }))
        if ($isCandidate -or $hostPid) {
            Write-Host $line -ForegroundColor $color
            $found = $true
        }
    }

    if (-not $found) {
        # 兜底：BTHLEDevice 下有没有目标 VID/PID 的设备键（哪怕没有诊断信息）
        Write-Host '--- fallback: BTHLEDevice keys (no WUDFDiagnosticInfo found) ---' -ForegroundColor DarkGray
        Get-ChildItem $base -ErrorAction SilentlyContinue | ForEach-Object {
            $name = $_.PSChildName
            if ($name -match '2717|32b8|1812|remote|mi|xiaomi|arn') {
                Write-Host ("KEY  " + $name) -ForegroundColor Yellow
                Get-ChildItem $_.PSPath -ErrorAction SilentlyContinue | ForEach-Object {
                    Write-Host ("  INST  " + $_.PSChildName) -ForegroundColor DarkYellow
                    $dp = Join-Path $_.PSPath 'Device Parameters'
                    if (Test-Path $dp) {
                        Write-Host ("    DP    present") -ForegroundColor DarkGray
                        $wd = Join-Path $dp 'WUDFDiagnosticInfo'
                        if (Test-Path $wd) {
                            $hp = (Get-ItemProperty -Path $wd -Name HostPid -ErrorAction SilentlyContinue).HostPid
                            Write-Host ("    WUDF  HostPid=" + $(if ($hp) { $hp } else { '-' })) -ForegroundColor Cyan
                        } else {
                            Write-Host ("    WUDF  (missing)") -ForegroundColor DarkGray
                        }
                    } else {
                        Write-Host ("    DP    (missing)") -ForegroundColor DarkGray
                    }
                }
                $found = $true
            }
        }
    }

    if (-not $found) {
        Write-Host 'NO-GO: no BTHLEDevice instance with WUDFDiagnosticInfo\HostPid.' -ForegroundColor Red
    }
}

# ---------------------------------------------------------------------------
# 3) wudfhost 进程一览（对照用）
# ---------------------------------------------------------------------------
Write-Section 'wudfhost processes'
Get-Process -Name wudfhost -ErrorAction SilentlyContinue |
    Select-Object Id, ProcessName, Path |
    Format-Table -AutoSize | Out-String -Width 300 | Write-Host

# ---------------------------------------------------------------------------
# 结论
# ---------------------------------------------------------------------------
Write-Section '结论判读'
Write-Host 'GO    : 某条 HW= 行 HostPid>0 且 Proc=wudfhost' -ForegroundColor Yellow
Write-Host '        -> 记下该行的 HostPid，进入第 2 步 Frida 探针。' -ForegroundColor Yellow
Write-Host 'PID?  : 有 HostPid 但进程名不是 wudfhost（PID 过期或非 WUDF 路径）' -ForegroundColor Yellow
Write-Host '        -> 记录后回报，可能需要重新插拔遥控器刷新诊断信息。' -ForegroundColor Yellow
Write-Host 'NO    : 所有条目都没有 HostPid' -ForegroundColor Yellow
Write-Host '        -> NO-GO：遥控器不走 WUDFHost，Frida 方案前提不成立，停止。' -ForegroundColor Yellow
