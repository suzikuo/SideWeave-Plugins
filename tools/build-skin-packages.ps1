param(
    [string]$SkinId = "",
    [string]$OutputDir = "./dist/packages",
    [switch]$ListOnly
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path $PSScriptRoot -Parent
$SkinRoot = Join-Path $RepoRoot "skins"
$OutputRoot = if ([IO.Path]::IsPathRooted($OutputDir)) {
    [IO.Path]::GetFullPath($OutputDir)
} else {
    [IO.Path]::GetFullPath((Join-Path $RepoRoot $OutputDir))
}

$Skins = @(
    foreach ($directory in Get-ChildItem -LiteralPath $SkinRoot -Directory | Sort-Object Name) {
        $manifestPath = Join-Path $directory.FullName "manifest.json"
        if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { continue }
        $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
        if ([int]$manifest.manifestVersion -ne 1 -or [int]$manifest.apiVersion -ne 1) {
            throw "Skin $($manifest.id) must declare manifestVersion and apiVersion as 1."
        }
        if ([string]$manifest.id -ne $directory.Name) {
            throw "Skin directory and manifest ID differ: $($directory.FullName)"
        }
        if ([string]$manifest.id -notmatch '^[a-z0-9][a-z0-9_-]*$') {
            throw "Invalid skin ID: $($manifest.id)"
        }
        if ([string]$manifest.version -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$') {
            throw "Invalid SemVer version for skin $($manifest.id): $($manifest.version)"
        }
        [PSCustomObject]@{
            Id = [string]$manifest.id
            Name = [string]$manifest.name
            Version = [string]$manifest.version
            Manifest = $manifest
            ManifestPath = $manifestPath
            SkinRoot = $directory.FullName
        }
    }
)

if ($SkinId) {
    $Skins = @($Skins | Where-Object { $_.Id -eq $SkinId })
    if ($Skins.Count -ne 1) { throw "Official skin not found: $SkinId" }
}
if ($Skins.Count -eq 0) { throw "No official skin packages were found." }

if ($ListOnly) {
    $choices = @($Skins | Sort-Object Id | ForEach-Object {
        [PSCustomObject]@{ id = $_.Id; name = $_.Name; version = $_.Version; kind = "skin" }
    })
    Write-Output (ConvertTo-Json -InputObject $choices -Depth 4 -Compress)
    return
}

$keyId = if ($env:SIDEWEAVE_OFFICIAL_PUBLISHER_KEY_ID) { $env:SIDEWEAVE_OFFICIAL_PUBLISHER_KEY_ID } else { "" }
foreach ($skin in $Skins) {
    $manifest = $skin.Manifest
    $packageKeyId = if ($keyId) { $keyId } else { [string]$manifest.key_id }
    if (-not $packageKeyId) { throw "Set SIDEWEAVE_OFFICIAL_PUBLISHER_KEY_ID before packaging official skin $($skin.Id)." }

    $temporaryRoot = Join-Path ([IO.Path]::GetTempPath()) ("sideweave-skin-" + [Guid]::NewGuid().ToString("N"))
    $packageRoot = Join-Path $temporaryRoot "package"
    New-Item -ItemType Directory -Path $packageRoot -Force | Out-Null
    try {
        Get-ChildItem -LiteralPath $skin.SkinRoot -Force | Copy-Item -Destination $packageRoot -Recurse -Force
        $packageManifestPath = Join-Path $packageRoot "manifest.json"
        $packageManifest = Get-Content -LiteralPath $packageManifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
        $packageManifest | Add-Member -NotePropertyName type -NotePropertyValue "skin" -Force
        $packageManifest | Add-Member -NotePropertyName publisher -NotePropertyValue "sideweave" -Force
        $packageManifest | Add-Member -NotePropertyName key_id -NotePropertyValue $packageKeyId -Force
        $packageManifest | Add-Member -NotePropertyName requires_trust -NotePropertyValue $false -Force
        $packageManifest | Add-Member -NotePropertyName capabilities -NotePropertyValue @() -Force
        [IO.File]::WriteAllText($packageManifestPath, ($packageManifest | ConvertTo-Json -Depth 32), [System.Text.UTF8Encoding]::new($false))

        if ($packageManifest.skinUi) {
            $assetRoot = [string]$packageManifest.skinUi.assetRoot
            if (-not $assetRoot -or [IO.Path]::IsPathRooted($assetRoot) -or $assetRoot -match '(^|[\\/])\.\.([\\/]|$)' -or $assetRoot -match '[:*?"<>|]') {
                throw "Skin $($skin.Id) has an unsafe skinUi.assetRoot."
            }
            foreach ($surface in @("sidebar", "detail", "control", "preview")) {
                $entry = [string]$packageManifest.skinUi.$surface
                if (-not $entry -or [IO.Path]::IsPathRooted($entry) -or $entry -match '(^|[\\/])\.\.([\\/]|$)' -or $entry -match '[:*?"<>|]') {
                    throw "Skin $($skin.Id) has an unsafe or missing skinUi.$surface entry."
                }
                $path = Join-Path (Join-Path $packageRoot $assetRoot) ($entry.Replace('/', '\'))
                if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Skin $($skin.Id) is missing skinUi.$surface entry: $entry" }
            }
        } elseif (-not (Test-Path -LiteralPath (Join-Path $packageRoot "skin.css") -PathType Leaf)) {
            throw "Skin $($skin.Id) must contain skin.css or a complete skinUi definition."
        }

        New-Item -ItemType Directory -Path $OutputRoot -Force | Out-Null
        $archive = Join-Path $OutputRoot "$($skin.Id)-$($skin.Version).zip"
        $stagingArchive = Join-Path $OutputRoot (".$($skin.Id)-$([Guid]::NewGuid().ToString('N')).tmp.zip")
        try {
            Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction Stop
            [System.IO.Compression.ZipFile]::CreateFromDirectory($packageRoot, $stagingArchive, [System.IO.Compression.CompressionLevel]::Optimal, $false)
            Move-Item -LiteralPath $stagingArchive -Destination $archive -Force
            Get-ChildItem -LiteralPath $OutputRoot -Filter "$($skin.Id)-*.zip" -File |
                Where-Object { $_.FullName -ne $archive } |
                Remove-Item -Force
        } finally {
            if (Test-Path -LiteralPath $stagingArchive) { Remove-Item -LiteralPath $stagingArchive -Force -ErrorAction SilentlyContinue }
        }
        Write-Host "Created skin package: $archive"
    } finally {
        if (Test-Path -LiteralPath $temporaryRoot) { Remove-Item -LiteralPath $temporaryRoot -Recurse -Force }
    }
}
