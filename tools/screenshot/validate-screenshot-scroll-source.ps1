[CmdletBinding()]
param(
    [switch]$RunRustChecks
)

$ErrorActionPreference = "Stop"
$sourceRoot = Join-Path $PSScriptRoot "..\..\plugins\screenshot-core\src"
$sourceFiles = @(Get-ChildItem -LiteralPath $sourceRoot -Filter "*.rs" -File)
if ($sourceFiles.Count -eq 0) {
    throw "screenshot-core source directory is empty: $sourceRoot"
}

$sourceText = ($sourceFiles | ForEach-Object { Get-Content -LiteralPath $_.FullName -Raw }) -join "`n"
$forbidden = [ordered]@{
    "mouse-wheel message path" = "WM_MOUSEWHEEL|WM_MOUSEHWHEEL"
    "legacy capture timer" = "TIMER_SCROLL_CAPTURE"
    "fixed 260ms/120ms algorithm" = "260\s*ms|120\s*ms"
    "scroll-specific control labels" = "自动滚动|手动滚动|滚动完成|滚动取消"
}

foreach ($entry in $forbidden.GetEnumerator()) {
    if ([regex]::IsMatch($sourceText, $entry.Value)) {
        throw "forbidden $($entry.Key) marker found in screenshot-core source"
    }
    Write-Output "source invariant passed: $($entry.Key)"
}

$utf8MojibakePatterns = @(
    ,[byte[]](0xE9, 0x8F, 0x83)
    ,[byte[]](0xE7, 0x92, 0x87)
    ,[byte[]](0xE9, 0x93, 0x8F)
    ,[byte[]](0xE5, 0xA6, 0x97)
    ,[byte[]](0xE5, 0xB1, 0x84)
    ,[byte[]](0xE9, 0x8D, 0xA5)
    ,[byte[]](0xE6, 0x83, 0xA7)
    ,[byte[]](0xE5, 0x84, 0x9A)
    ,[byte[]](0xE9, 0x96, 0xBF)
)
foreach ($file in $sourceFiles) {
    $bytes = [IO.File]::ReadAllBytes($file.FullName)
    foreach ($pattern in $utf8MojibakePatterns) {
        for ($i = 0; $i -le $bytes.Length - $pattern.Length; $i++) {
            $matched = $true
            for ($j = 0; $j -lt $pattern.Length; $j++) {
                if ($bytes[$i + $j] -ne $pattern[$j]) {
                    $matched = $false
                    break
                }
            }
            if ($matched) {
                throw ("forbidden known mojibake UTF-8 sequence found in $($file.Name)")
            }
        }
    }
}
Write-Output "source invariant passed: known mojibake"

$required = @(
    "SCROLL_OBSERVE_INTERVAL",
    "SETTLING_OBSERVATIONS",
    "CapturePhase::Settling",
    "FrameResult::NoOverlap",
    "FrameResult::ReverseDirection",
    "MAX_SCROLL_HEIGHT",
    "scroll.seed(selection.width(), selection.height(), frame)",
    "scroll.seed_probe(screen.probe(selection))",
    "paint_scroll_preview",
    "let title_text = status_error",
    "InvalidateRect(state.scroll_preview",
    "grab_probe(selection)",
    "grab_region(selection)",
    "pause_after_capture_failure",
    "scroll.ready_to_capture()",
    "Some(ToolbarAction::Scroll) => start_scroll(state)",
    "Some(ToolbarAction::Copy) => copy_capture(state)",
    "Some(ToolbarAction::Save) => finish_with_dialog(state)",
    "Some(ToolbarAction::Cancel) => cancel(state)",
    "Some(ToolbarAction::Finish) => finish(state)"
)
foreach ($literal in $required) {
    if (-not $sourceText.Contains($literal)) {
        throw "required screenshot-core path is missing: $literal"
    }
    Write-Output "source path present: $literal"
}

$borderForwarding = '(?s)unsafe fn scroll_border_wnd_proc.*?DefWindowProcW\(hwnd, message, wparam, lparam\)'
if (-not [regex]::IsMatch($sourceText, $borderForwarding)) {
    throw "scroll border window must forward real DefWindowProcW parameters"
}
Write-Output "source invariant passed: scroll-border DefWindowProc forwarding"
$observerBody = [regex]::Match($sourceText, "(?s)fn scroll_tick\(.*?(?=\r?\nfn scroll_capture_tick)")
if (-not $observerBody.Success) { throw "scroll observer function was not found" }
if (-not $observerBody.Value.Contains("ScreenCapture::grab_probe(selection)")) { throw "scroll observer must use grab_probe" }
if ($observerBody.Value.Contains("ScreenCapture::grab_region(selection)")) { throw "scroll observer must not use grab_region" }
Write-Output "source invariant passed: observer uses probe-only capture"
$stableBody = [regex]::Match($sourceText, "(?s)fn scroll_capture_tick\(.*?(?=\r?\nfn save_png_with_dialog)")
if (-not $stableBody.Success -or -not $stableBody.Value.Contains("ScreenCapture::grab_region(selection)")) { throw "stable capture path must use grab_region" }
Write-Output "source invariant passed: stable capture uses region capture"
$startBody = [regex]::Match($sourceText, "(?s)fn start_scroll\(.*?(?=\r?\nfn scroll_tick)")
if (-not $startBody.Success) { throw "start_scroll function was not found" }
$seedFrameIndex = $startBody.Value.IndexOf("scroll.seed(selection.width(), selection.height(), frame)")
$seedProbeIndex = $startBody.Value.IndexOf("scroll.seed_probe(screen.probe(selection))")
if ($seedFrameIndex -lt 0 -or $seedProbeIndex -lt 0 -or $seedFrameIndex -ge $seedProbeIndex) { throw "start_scroll must seed the first frame before the initial probe" }
Write-Output "source invariant passed: first-frame seed precedes initial probe"

if ($RunRustChecks) {
    Push-Location (Join-Path $PSScriptRoot "..\..")
    try {
        cargo fmt --manifest-path plugins/screenshot-core/Cargo.toml -- --check
        if ($LASTEXITCODE -ne 0) { throw "cargo fmt check failed" }
        cargo clippy --manifest-path plugins/screenshot-core/Cargo.toml --all-targets -- -D warnings
        if ($LASTEXITCODE -ne 0) { throw "cargo clippy failed" }
    } finally {
        Pop-Location
    }
}

Write-Output "screenshot-scroll-source-validation=passed"
