# Shared preflight and transactional version updates for publish-wizard.ps1.

function Get-NextPackageVersion([string]$Current, [string[]]$ExistingVersions) {
    $highest = [version](($Current -split '[-+]')[0])
    foreach ($versionText in $ExistingVersions) {
        $core = [version](($versionText -split '[-+]')[0])
        if ($core -gt $highest) { $highest = $core }
    }
    return '{0}.{1}.{2}' -f $highest.Major, $highest.Minor, ($highest.Build + 1)
}

function Get-PackageVersionPlan([object[]]$Plugins, [bool]$LocalMode, [string]$Commit) {
    $entries = @()
    $dirtySource = $LocalMode -and [bool](Get-GitText $RepoRoot @('status', '--porcelain'))
    foreach ($plugin in $Plugins) {
        $prefix = "$($plugin.kind)-$($plugin.id)-v"
        $tag = "$prefix$($plugin.version)"
        $pattern = '^' + [regex]::Escape($prefix) + '(?<version>(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?)$'
        $tags = @{}
        if ($LocalMode) {
            foreach ($localTag in @((Get-GitText $RepoRoot @('tag', '--list', "$prefix*")) -split '\r?\n')) {
                if ($localTag -match $pattern) {
                    $tags[$localTag] = Get-GitText $RepoRoot @('rev-parse', "$localTag^{}")
                }
            }
        }
        # A local build can still detect published versions without requiring gh login.
        $remote = if ($LocalMode) { 'origin' } else { "https://github.com/$Repository.git" }
        $query = Invoke-GitCapture $RepoRoot @('ls-remote', '--tags', $remote, "refs/tags/$prefix*")
        if ($query.ExitCode -ne 0) {
            if (-not $LocalMode) { throw "无法检查 $($plugin.id) 的远程版本：$(@($query.Output) -join [Environment]::NewLine)" }
            Write-Warning "无法读取 $($plugin.id) 的远程 tags；本地打包仅检查本地 tags。"
        }
        else {
            # Annotated tags have both object and peeled commit rows; the latter wins.
            foreach ($peeled in @($false, $true)) {
                foreach ($line in $query.Output) {
                    if ([string]$line -notmatch '^(?<sha>[0-9a-f]+)\s+refs/tags/(?<tag>.+)$') { continue }
                    $sha = $Matches.sha
                    $remoteTag = $Matches.tag
                    $isPeeled = $remoteTag.EndsWith('^{}', [StringComparison]::Ordinal)
                    if ($isPeeled -ne $peeled) { continue }
                    if ($isPeeled) { $remoteTag = $remoteTag.Substring(0, $remoteTag.Length - 3) }
                    if ($remoteTag -match $pattern) { $tags[$remoteTag] = $sha }
                }
            }
        }
        $versions = @(foreach ($existingTag in $tags.Keys) {
            if ($existingTag -match $pattern) { $Matches.version }
        })
        $reason = ''
        if ($tags.ContainsKey($tag) -and ($tags[$tag] -ne $Commit -or $dirtySource)) { $reason = "tag '$tag' 已存在，当前源码需要使用新版本" }
        try { Assert-PluginVersionDoesNotRegress -PluginId $plugin.id -Candidate $plugin.version -ExistingVersions $versions }
        catch { $reason = $_.Exception.Message }
        $entries += [PSCustomObject]@{ Plugin = $plugin; Tag = $tag; Occupied = $tags.ContainsKey($tag); Versions = $versions; Reason = $reason }
    }
    if (@($entries | Where-Object { $_.Reason }).Count -eq 0) { return }
    foreach ($entry in $entries) {
        # A bump creates a new source commit, so other selected, already-tagged
        # versions must also advance even when their tags currently equal HEAD.
        if (-not $entry.Reason -and -not $entry.Occupied) { continue }
        [PSCustomObject]@{
            Plugin = $entry.Plugin
            OldVersion = [string]$entry.Plugin.version
            NewVersion = Get-NextPackageVersion $entry.Plugin.version $entry.Versions
            Reason = if ($entry.Reason) { $entry.Reason } else { '批量升级后源码提交会改变，当前版本的 tag 已存在' }
        }
    }
}

function Update-PackageVersions([object[]]$Plan) {
    $updates = @{}
    $originals = @{}
    $hasNative = $false
    foreach ($item in $Plan) {
        $plugin = $item.Plugin
        $root = Join-Path $RepoRoot $(if ($plugin.kind -eq 'skin') { 'skins' } else { 'plugins' })
        $manifests = @(foreach ($file in Get-ChildItem -LiteralPath $root -Filter manifest.json -File -Recurse) {
            $manifest = Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8 | ConvertFrom-Json
            if ($manifest.id -eq $plugin.id) { $file }
        })
        if ($manifests.Count -ne 1) { throw "无法唯一定位 $($plugin.id) 的 manifest.json。" }
        $manifestPath = $manifests[0].FullName
        $text = [IO.File]::ReadAllText($manifestPath)
        $manifestVersion = [regex]::new('("version"\s*:\s*")' + [regex]::Escape($item.OldVersion) + '(")')
        if ($manifestVersion.Matches($text).Count -ne 1) { throw "无法安全修改 $manifestPath 的版本字段。" }
        $updates[$manifestPath] = $manifestVersion.Replace($text, '${1}' + $item.NewVersion + '${2}')
        if ($plugin.kind -ne 'plugin') { continue }
        $hasNative = $true
        $cargoPath = Join-Path $manifests[0].DirectoryName 'Cargo.toml'
        $cargo = [IO.File]::ReadAllText($cargoPath)
        $section = [regex]::Match($cargo, '(?ms)^\[package\][^\r\n]*\r?\n(?<body>.*?)(?=^\[|\z)')
        if (-not $section.Success) { throw "找不到 $cargoPath 的 [package]。" }
        $body = $section.Groups['body'].Value
        $versionField = [regex]::new('(?m)^(?<indent>[^\S\r\n]*)version\s*=\s*"' + [regex]::Escape($item.OldVersion) + '"(?<tail>[^\r\n]*)(?=\r?$)')
        $inheritedField = [regex]::new('(?m)^(?<indent>[^\S\r\n]*)version\.workspace\s*=\s*true(?<tail>[^\r\n]*)(?=\r?$)')
        if ($versionField.Matches($body).Count -eq 1) {
            $newBody = $versionField.Replace($body, '${indent}version = "' + $item.NewVersion + '"${tail}')
        }
        elseif ($inheritedField.Matches($body).Count -eq 1) {
            # Make this package independent instead of changing every workspace member.
            $newBody = $inheritedField.Replace($body, '${indent}version = "' + $item.NewVersion + '"${tail}')
        }
        else { throw "无法安全修改 $cargoPath 的版本字段。" }
        $bodyGroup = $section.Groups['body']
        $updates[$cargoPath] = $cargo.Substring(0, $bodyGroup.Index) + $newBody + $cargo.Substring($bodyGroup.Index + $bodyGroup.Length)
    }
    $lockPath = Join-Path $RepoRoot 'Cargo.lock'
    $paths = @($updates.Keys)
    if ($hasNative) { $paths += $lockPath }
    foreach ($path in $paths) {
        $originals[$path] = if (Test-Path -LiteralPath $path) { [IO.File]::ReadAllBytes($path) } else { $null }
    }
    try {
        foreach ($path in $updates.Keys) {
            $bytes = $originals[$path]
            $bom = $bytes.Length -ge 3 -and $bytes[0] -eq 239 -and $bytes[1] -eq 187 -and $bytes[2] -eq 191
            [IO.File]::WriteAllText($path, $updates[$path], [Text.UTF8Encoding]::new($bom))
        }
        if ($hasNative) {
            # Resolve only local package version changes; never update SDK revisions.
            $null = & cargo update --manifest-path (Join-Path $RepoRoot 'Cargo.toml') --workspace --offline --quiet
            if ($LASTEXITCODE -ne 0) { throw '同步 Cargo.lock 失败，已还原本次版本改动。' }
        }
    }
    catch {
        foreach ($path in $originals.Keys) {
            if ($null -eq $originals[$path]) { Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue }
            else { [IO.File]::WriteAllBytes($path, $originals[$path]) }
        }
        throw
    }
    foreach ($item in $Plan) { $item.Plugin.version = $item.NewVersion }
    return $paths
}
