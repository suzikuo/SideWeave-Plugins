[CmdletBinding()]
param(
    [switch]$SkipRustTests,
    [switch]$Release,
    [switch]$StrictMatrix
)

$ErrorActionPreference = "Stop"

function Write-Section([string]$Title) {
    Write-Output ""
    Write-Output "=== $Title ==="
}

Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class ScreenshotDisplayNative {
    [StructLayout(LayoutKind.Sequential)]
    public struct Point {
        public int X;
        public int Y;
        public Point(int x, int y) { X = x; Y = y; }
    }

    [DllImport("user32.dll")]
    public static extern bool SetProcessDpiAwarenessContext(IntPtr context);

    [DllImport("user32.dll")]
    public static extern uint GetDpiForSystem();

    [DllImport("user32.dll")]
    private static extern IntPtr MonitorFromPoint(Point point, uint flags);

    [DllImport("shcore.dll")]
    private static extern int GetDpiForMonitor(IntPtr monitor, int dpiType, out uint dpiX, out uint dpiY);

    public static uint[] GetEffectiveMonitorDpi(int x, int y) {
        var monitor = MonitorFromPoint(new Point(x, y), 2 /* MONITOR_DEFAULTTONEAREST */);
        if (monitor == IntPtr.Zero) return null;
        uint dpiX;
        uint dpiY;
        if (GetDpiForMonitor(monitor, 0 /* MDT_EFFECTIVE_DPI */, out dpiX, out dpiY) != 0) return null;
        return new[] { dpiX, dpiY };
    }
}
"@

# Read physical monitor bounds before loading WinForms so the host is not DPI-virtualized.
[ScreenshotDisplayNative]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null
Add-Type -AssemblyName System.Windows.Forms

Write-Section "Physical display baseline"
$screens = [System.Windows.Forms.Screen]::AllScreens
if ($null -eq $screens -or $screens.Count -eq 0) {
    throw "Windows did not report a physical display."
}

$monitorDpiValues = @()

foreach ($screen in $screens) {
    $bounds = $screen.Bounds
    $workArea = $screen.WorkingArea
    $primary = if ($screen.Primary) { "primary" } else { "secondary" }
    $monitorDpi = [ScreenshotDisplayNative]::GetEffectiveMonitorDpi($bounds.X + 1, $bounds.Y + 1)
    if ($null -ne $monitorDpi) {
        $monitorDpiValues += @([uint32]$monitorDpi[0], [uint32]$monitorDpi[1])
    }
    $dpiText = if ($null -ne $monitorDpi) {
        "effectiveDPI={0}x{1} ({2}%/{3}%)" -f $monitorDpi[0], $monitorDpi[1],
            [math]::Round(($monitorDpi[0] / 96.0) * 100), [math]::Round(($monitorDpi[1] / 96.0) * 100)
    } else {
        "effectiveDPI=unavailable"
    }
    Write-Output ("{0}: {1}, bounds=({2},{3}) {4}x{5}, workArea=({6},{7}) {8}x{9}, {10}" -f `
        $screen.DeviceName, $primary, $bounds.X, $bounds.Y, $bounds.Width, $bounds.Height,
        $workArea.X, $workArea.Y, $workArea.Width, $workArea.Height, $dpiText)
}

$dpi = [ScreenshotDisplayNative]::GetDpiForSystem()
if ($dpi -eq 0) {
    Write-Warning "GetDpiForSystem returned 0; system DPI evidence is unavailable."
} else {
    $scale = [math]::Round(($dpi / 96.0) * 100, 1)
    Write-Output ("system DPI={0} ({1}% scale)" -f $dpi, $scale)
}

$targetDpi = @(96, 120, 144, 168, 192)
$uniqueMonitorDpi = @($monitorDpiValues | Sort-Object -Unique)
if ($uniqueMonitorDpi.Count -gt 0) {
    Write-Output ("observed per-monitor effective DPI: {0}" -f (($uniqueMonitorDpi | ForEach-Object { "$($_)" }) -join ", "))
}
$unexpectedMonitorDpi = @($uniqueMonitorDpi | Where-Object { $targetDpi -notcontains $_ })
$targetScale = $targetDpi | ForEach-Object { "{0}%" -f [math]::Round(($_ / 96.0) * 100) }
$observedScale = if ($dpi -gt 0) { "{0}%" -f [math]::Round(($dpi / 96.0) * 100) } else { "unknown" }
Write-Output ("target hardware scale matrix: {0}" -f ($targetScale -join ", "))
Write-Output ("observed system scale: {0}" -f $observedScale)

if ($screens.Count -lt 2) {
    Write-Warning "Only one physical display is available; multi-monitor and negative-origin behavior still require hardware validation."
}
if ($dpi -gt 0 -and $targetDpi -notcontains $dpi) {
    Write-Warning "The current system DPI is outside the requested 100%/125%/150%/175%/200% matrix."
}
if ($unexpectedMonitorDpi.Count -gt 0) {
    Write-Warning ("Observed per-monitor DPI outside the requested matrix: {0}" -f ($unexpectedMonitorDpi -join ", "))
}

Write-Section "Synthetic geometry regression"
if ($SkipRustTests) {
    Write-Output "Skipped Rust geometry tests (-SkipRustTests)."
} else {
    $profile = if ($Release) { "--release" } else { $null }
    $testArgs = @("test", "--manifest-path", "plugins/screenshot-core/Cargo.toml")
    if ($null -ne $profile) { $testArgs += $profile }
    $testArgs += @("layout_tests", "--", "--nocapture")
    Write-Output ("cargo {0}" -f ($testArgs -join " "))
    & cargo @testArgs
    if ($LASTEXITCODE -ne 0) {
        throw "Synthetic geometry regression failed with exit code $LASTEXITCODE."
    }
    Write-Output "Synthetic geometry regression passed."
}

Write-Section "Matrix result"
$strictDpi = if ($uniqueMonitorDpi.Count -gt 0) {
    $unexpectedMonitorDpi.Count -eq 0
} else {
    $dpi -gt 0 -and $targetDpi -contains $dpi
}
if ($screens.Count -ge 2 -and $strictDpi) {
    Write-Output "Hardware baseline covers multiple displays and one requested DPI scale; remaining requested scales still need separate runs."
} else {
    Write-Output "Code-level geometry coverage is recorded separately from hardware coverage; keep P1 multi-display/DPI validation pending."
}

if ($StrictMatrix -and ($screens.Count -lt 2 -or -not $strictDpi)) {
    throw "Strict matrix mode requires at least two displays and all observed monitor DPIs to be in the requested scale matrix."
}
