param(
    [ValidateSet("debug", "release")]
    [string]$BuildProfile = "release",
    [string]$OutputDir = "./dist/packages",
    [string]$PluginId = "",
    [switch]$ListOnly
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path $PSScriptRoot -Parent
$ProfileDir = if ($BuildProfile -eq "release") { "release" } else { "debug" }
$OutputRoot = if ([IO.Path]::IsPathRooted($OutputDir)) {
    [IO.Path]::GetFullPath($OutputDir)
} else {
    [IO.Path]::GetFullPath((Join-Path $RepoRoot $OutputDir))
}
$Vite = Join-Path $RepoRoot "node_modules\.bin\vite.cmd"
$ViteConfig = Join-Path $PSScriptRoot "plugin-ui.vite.config.ts"
$ManifestFiles = @(Get-ChildItem -LiteralPath (Join-Path $RepoRoot "plugins") -Filter "manifest.json" -File -Recurse)
$MetadataJson = & cargo metadata --manifest-path (Join-Path $RepoRoot "Cargo.toml") --no-deps --format-version 1 --quiet
if ($LASTEXITCODE -ne 0) { throw "Could not read Cargo workspace metadata." }
$Metadata = (@($MetadataJson) -join [Environment]::NewLine) | ConvertFrom-Json

$Plugins = @(
    foreach ($manifestFile in $ManifestFiles) {
        $manifest = Get-Content -LiteralPath $manifestFile.FullName -Raw -Encoding UTF8 | ConvertFrom-Json
        if ([string]$manifest.publisher -ne "sideweave") { continue }
        $cargoManifest = Join-Path $manifestFile.DirectoryName "Cargo.toml"
        $package = @($Metadata.packages | Where-Object {
            [StringComparer]::OrdinalIgnoreCase.Equals(
                [IO.Path]::GetFullPath([string]$_.manifest_path),
                [IO.Path]::GetFullPath($cargoManifest)
            )
        }) | Select-Object -First 1
        if (-not $package) { continue }
        if ([string]$manifest.version -ne [string]$package.version) {
            throw "Cargo and manifest versions differ for $($manifest.id): $($package.version) vs $($manifest.version)"
        }
        if ([string]$manifest.type -ne "native") { throw "Unsupported official package type for $($manifest.id)." }
        if ([string]$manifest.runtime -notin @("dynamic_library", "external_process")) {
            throw "Unsupported native runtime for $($manifest.id)."
        }
        [PSCustomObject]@{
            Id = [string]$manifest.id
            Name = [string]$manifest.name
            Version = [string]$manifest.version
            Manifest = $manifest
            ManifestPath = $manifestFile.FullName
            PluginRoot = $manifestFile.DirectoryName
            CargoPackage = $package
        }
    }
)

if ($PluginId) {
    $Plugins = @($Plugins | Where-Object { $_.Id -eq $PluginId })
    if ($Plugins.Count -ne 1) { throw "Official plugin not found: $PluginId" }
}
if ($Plugins.Count -eq 0) { throw "No official plugin packages were found." }

if ($ListOnly) {
    $choices = @($Plugins | Sort-Object Id | ForEach-Object {
        [PSCustomObject]@{ id = $_.Id; name = $_.Name; version = $_.Version; kind = "plugin" }
    })
    Write-Output (ConvertTo-Json -InputObject $choices -Depth 4 -Compress)
    return
}

if (-not (Test-Path -LiteralPath $Vite -PathType Leaf)) {
    throw "Vite not found at $Vite. Run npm ci in the plugin repository."
}
New-Item -ItemType Directory -Path $OutputRoot -Force | Out-Null

function Get-SafeRelativePath([string]$Path, [string]$Field) {
    if ([string]::IsNullOrWhiteSpace($Path) -or [IO.Path]::IsPathRooted($Path) -or $Path -match '(^|[\\/])\.\.([\\/]|$)' -or $Path -match '[:*?"<>|]') {
        throw "$Field must be a safe package-relative path: $Path"
    }
    return $Path.Replace('/', '\')
}

function Restore-EnvironmentValue([string]$Name, [object]$Value) {
    if ($null -eq $Value) { Remove-Item "Env:$Name" -ErrorAction SilentlyContinue }
    else { Set-Item "Env:$Name" ([string]$Value) }
}

foreach ($plugin in $Plugins) {
    $manifest = $plugin.Manifest
    $targets = @($plugin.CargoPackage.targets)
    if ($manifest.runtime -eq "dynamic_library") {
        $target = @($targets | Where-Object { @($_.crate_types) -contains "cdylib" }) | Select-Object -First 1
        if (-not $target) { throw "No cdylib Cargo target for $($plugin.Id)." }
        $artifactName = "$($target.name).dll"
        $packageBackend = Get-SafeRelativePath ([string]$manifest.backend) "backend"
    } else {
        $backendPath = Get-SafeRelativePath ([string]$manifest.backend) "backend"
        $target = @($targets | Where-Object {
            @($_.kind) -contains "bin" -and "$($_.name).exe" -eq [IO.Path]::GetFileName($backendPath)
        }) | Select-Object -First 1
        if (-not $target) { throw "No binary Cargo target matching manifest backend for $($plugin.Id)." }
        $artifactName = "$($target.name).exe"
        $packageBackend = $backendPath
    }

    Write-Host "Building $($plugin.Name) [$($plugin.Id)] v$($plugin.Version)"
    $cargoArgs = @("build", "--manifest-path", (Join-Path $RepoRoot "Cargo.toml"), "-p", [string]$plugin.CargoPackage.name)
    if ($BuildProfile -eq "release") { $cargoArgs += "--release" }
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw "Cargo build failed for $($plugin.Id)." }

    $artifactPath = Join-Path $RepoRoot "target\$ProfileDir\$artifactName"
    if (-not (Test-Path -LiteralPath $artifactPath -PathType Leaf)) { throw "Plugin artifact not found: $artifactPath" }

    $temporaryRoot = Join-Path ([IO.Path]::GetTempPath()) ("sideweave-plugin-" + [Guid]::NewGuid().ToString("N"))
    $packageRoot = Join-Path $temporaryRoot $plugin.Id
    New-Item -ItemType Directory -Path $packageRoot -Force | Out-Null
    try {
        Copy-Item -LiteralPath $plugin.ManifestPath -Destination (Join-Path $packageRoot "manifest.json")
        $noticeSource = Join-Path $plugin.PluginRoot "THIRD_PARTY_NOTICES.md"
        if (Test-Path -LiteralPath $noticeSource -PathType Leaf) {
            Copy-Item -LiteralPath $noticeSource -Destination (Join-Path $packageRoot "THIRD_PARTY_NOTICES.md")
        }
        $backendDestination = Join-Path $packageRoot $packageBackend
        New-Item -ItemType Directory -Path (Split-Path -Parent $backendDestination) -Force | Out-Null
        Copy-Item -LiteralPath $artifactPath -Destination $backendDestination -Force

        $assetRoot = Get-SafeRelativePath ([string]$manifest.ui.assetRoot) "ui.assetRoot"
        $uiSource = Join-Path $plugin.PluginRoot "ui"
        if (-not (Test-Path -LiteralPath $uiSource -PathType Container)) { throw "Plugin UI source directory missing: $uiSource" }
        $previousUiRoot = $env:AGILE_PLUGIN_UI_ROOT
        $previousUiOut = $env:AGILE_PLUGIN_UI_OUT
        try {
            $env:AGILE_PLUGIN_UI_ROOT = $uiSource
            $env:AGILE_PLUGIN_UI_OUT = Join-Path $packageRoot $assetRoot
            & $Vite build --config $ViteConfig --configLoader runner
            if ($LASTEXITCODE -ne 0) { throw "Vite build failed for $($plugin.Id)." }
        } finally {
            Restore-EnvironmentValue "AGILE_PLUGIN_UI_ROOT" $previousUiRoot
            Restore-EnvironmentValue "AGILE_PLUGIN_UI_OUT" $previousUiOut
        }

        $iconRelative = Get-SafeRelativePath ([string]$manifest.icon) "icon"
        $iconDestination = Join-Path $packageRoot $iconRelative
        if (-not (Test-Path -LiteralPath $iconDestination -PathType Leaf)) {
            $iconName = [IO.Path]::GetFileName($iconRelative)
            $iconSource = Join-Path $uiSource (Join-Path "public" $iconName)
            if (-not (Test-Path -LiteralPath $iconSource -PathType Leaf)) { $iconSource = Join-Path $uiSource $iconName }
            if (-not (Test-Path -LiteralPath $iconSource -PathType Leaf)) { throw "Plugin icon not found: $iconSource" }
            New-Item -ItemType Directory -Path (Split-Path -Parent $iconDestination) -Force | Out-Null
            Copy-Item -LiteralPath $iconSource -Destination $iconDestination -Force
        }

        $entryRelative = Get-SafeRelativePath ([string]$manifest.ui.entry) "ui.entry"
        if (-not (Test-Path -LiteralPath (Join-Path $packageRoot $entryRelative) -PathType Leaf)) {
            throw "Built plugin entry is missing: $entryRelative"
        }
        $archive = Join-Path $OutputRoot "$($plugin.Id)-$($plugin.Version).zip"
        $stagingArchive = Join-Path $OutputRoot (".$($plugin.Id)-$([Guid]::NewGuid().ToString('N')).tmp.zip")
        try {
            Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction Stop
            [System.IO.Compression.ZipFile]::CreateFromDirectory($packageRoot, $stagingArchive, [System.IO.Compression.CompressionLevel]::Optimal, $false)
            Move-Item -LiteralPath $stagingArchive -Destination $archive -Force
            Get-ChildItem -LiteralPath $OutputRoot -Filter "$($plugin.Id)-*.zip" -File |
                Where-Object { $_.FullName -ne $archive } |
                Remove-Item -Force
        } finally {
            if (Test-Path -LiteralPath $stagingArchive) { Remove-Item -LiteralPath $stagingArchive -Force -ErrorAction SilentlyContinue }
        }
        Write-Host "Created package: $archive"
    } finally {
        if (Test-Path -LiteralPath $temporaryRoot) { Remove-Item -LiteralPath $temporaryRoot -Recurse -Force }
    }
}
