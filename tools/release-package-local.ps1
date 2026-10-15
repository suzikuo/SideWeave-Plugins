param(
    [Parameter(Mandatory = $true)][string]$Id,
    [string]$Version = "",
    [string]$SigningKeyPath = "",
    [string]$MarketplacePath = "",
    [string]$MinisignExe = "",
    [string]$OutputPath = "dist/local-release"
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path $PSScriptRoot -Parent
$WorkspaceRoot = Split-Path $RepoRoot -Parent
if (-not $MarketplacePath) { $MarketplacePath = Join-Path $WorkspaceRoot "marketplace" }
if (-not $SigningKeyPath) { $SigningKeyPath = Join-Path $WorkspaceRoot "_private/sideweave-publisher.key" }
if (-not $MinisignExe) { $MinisignExe = Join-Path $WorkspaceRoot "_private/minisign/minisign.exe" }
foreach ($pathName in @("MarketplacePath", "SigningKeyPath", "MinisignExe")) {
    $value = Get-Variable -Name $pathName -ValueOnly
    if (-not [IO.Path]::IsPathRooted($value)) {
        Set-Variable -Name $pathName -Value (Join-Path $WorkspaceRoot $value)
    }
}
if (-not [IO.Path]::IsPathRooted($OutputPath)) { $OutputPath = Join-Path $RepoRoot $OutputPath }
$OutputPath = [IO.Path]::GetFullPath($OutputPath)
$manifestPath = $null
$manifest = $null
foreach ($candidate in Get-ChildItem -LiteralPath (Join-Path $RepoRoot "plugins") -Filter "manifest.json" -File -Recurse) {
    $candidateManifest = Get-Content -LiteralPath $candidate.FullName -Raw -Encoding UTF8 | ConvertFrom-Json
    if ([string]$candidateManifest.id -eq $Id -and [string]$candidateManifest.publisher -eq "sideweave") {
        if ($manifestPath) { throw "More than one official manifest uses plugin ID $Id." }
        $manifestPath = $candidate.FullName
        $manifest = $candidateManifest
    }
}
if (-not $manifestPath) { throw "Official plugin manifest not found for ID: $Id" }
if (-not (Test-Path -LiteralPath $SigningKeyPath -PathType Leaf)) { throw "Publisher private key not found: $SigningKeyPath" }
if (-not (Test-Path -LiteralPath $MinisignExe -PathType Leaf)) {
    $command = Get-Command minisign -ErrorAction SilentlyContinue
    if ($command) { $MinisignExe = $command.Source }
}
if (-not (Test-Path -LiteralPath $MinisignExe -PathType Leaf)) { throw "Minisign executable not found. Set -MinisignExe or add minisign to PATH." }

if ([string]$manifest.id -ne $Id) { throw "Manifest ID does not match $Id." }
if (-not $Version) { $Version = [string]$manifest.version }
if ([string]$manifest.version -ne $Version) { throw "Manifest version must match $Version." }
if ([string]$manifest.publisher -ne "sideweave" -or -not $manifest.key_id) {
    throw "Official plugin manifests must declare publisher sideweave and key_id."
}
if ($manifest.type -ne "native" -or $manifest.requires_trust -ne $true) {
    throw "Native plugin manifests must use type native and requires_trust true."
}

$publishersPath = Join-Path $MarketplacePath "publishers.json"
if (-not (Test-Path -LiteralPath $publishersPath -PathType Leaf)) { throw "Marketplace publishers.json not found: $publishersPath" }
$publishers = Get-Content -LiteralPath $publishersPath -Raw -Encoding UTF8 | ConvertFrom-Json
$official = $publishers.publishers | Where-Object { $_.id -eq "sideweave" } | Select-Object -First 1
$activeKeys = @($official.keys | Where-Object { $_.status -eq "active" })
if ($activeKeys.Count -ne 1) { throw "publishers.json must contain exactly one active SideWeave publisher key; found $($activeKeys.Count)." }
$keyRecord = $activeKeys[0]
if ([string]$manifest.key_id -ne [string]$keyRecord.id) {
    throw "Manifest key_id $($manifest.key_id) is not the active publisher key $($keyRecord.id)."
}
$keyHeader = [IO.File]::ReadLines($SigningKeyPath) | Select-Object -First 1
if ([string]$keyHeader -notmatch '^untrusted comment:.*secret key') {
    throw "Publisher key is not a complete Minisign secret-key file."
}

$tag = "plugin-$Id-v$Version"
$releaseRoot = Join-Path $OutputPath $tag
New-Item -ItemType Directory -Path $releaseRoot -Force | Out-Null
$derivedPublicKey = Join-Path $releaseRoot "publisher-derived.pub"
try {
    & $MinisignExe -R -s $SigningKeyPath -p $derivedPublicKey
    if ($LASTEXITCODE -ne 0) { throw "Could not derive public key from the signing key." }
    $derivedPayload = (Get-Content -LiteralPath $derivedPublicKey | Select-Object -Last 1).Trim()
    if ($derivedPayload -ne [string]$keyRecord.public_key) {
        throw "Signing key does not match sideweave/$($keyRecord.id) in publishers.json."
    }

    if (-not (Test-Path -LiteralPath (Join-Path $RepoRoot "node_modules\.bin\vite.cmd") -PathType Leaf)) {
        Push-Location $RepoRoot
        try {
            & npm ci
            if ($LASTEXITCODE -ne 0) { throw "npm ci failed with exit code $LASTEXITCODE." }
        } finally {
            Pop-Location
        }
    }
    $buildScript = Join-Path $RepoRoot "tools/build-plugin-packages.ps1"
    & $buildScript -PluginId $Id -BuildProfile release -OutputDir $releaseRoot
    if ($LASTEXITCODE -ne 0) { throw "Plugin build failed: $Id." }

    $archive = Join-Path $releaseRoot "$Id-$Version.zip"
    if (-not (Test-Path -LiteralPath $archive -PathType Leaf)) { throw "Package archive not found: $archive" }
    $signaturePath = "$archive.minisig"
    $checksumPath = "$archive.sha256"
    Remove-Item -LiteralPath $signaturePath, $checksumPath -Force -ErrorAction SilentlyContinue
    & $MinisignExe -S -s $SigningKeyPath -m $archive -x $signaturePath
    if ($LASTEXITCODE -ne 0) { throw "Minisign could not sign $([IO.Path]::GetFileName($archive))." }
    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    $checksumText = "$hash  $([IO.Path]::GetFileName($archive))" + [Environment]::NewLine
    [IO.File]::WriteAllText($checksumPath, $checksumText, (New-Object Text.UTF8Encoding($false)))
    Remove-Item -LiteralPath $derivedPublicKey -Force -ErrorAction SilentlyContinue

    Write-Host "Signed package assets are ready for Release tag $($tag):"
    Write-Host "  $archive"
    Write-Host "  $signaturePath"
    Write-Host "  $checksumPath"
} finally {
    Remove-Item -LiteralPath $derivedPublicKey -Force -ErrorAction SilentlyContinue
}
