# SideWeave-Plugins local package and GitHub Release publishing wizard.
# Run without arguments for the interactive workflow.

[CmdletBinding()]
param(
    [string]$Id = "",
    [string]$Version = "",
    [string]$Repository = "suzikuo/SideWeave-Plugins",
    [string]$MarketplacePath = "",
    [string]$SigningKeyPath = "",
    [string]$MinisignExe = "",
    [string]$Notes = "",
    [switch]$LocalOnly,
    [switch]$SkipBuild,
    [switch]$SkipMarketplaceUpdate,
    [switch]$PreviewOnly
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path $PSScriptRoot -Parent
$WorkspaceRoot = Split-Path $RepoRoot -Parent
$ReleaseBuildScript = Join-Path $RepoRoot "tools/release-package-local.ps1"
$PluginListScript = Join-Path $RepoRoot "tools/build-plugin-packages.ps1"
$SkinListScript = Join-Path $RepoRoot "tools/build-skin-packages.ps1"
$OutputRoot = Join-Path $RepoRoot "dist/local-release"
if (-not $MarketplacePath) { $MarketplacePath = Join-Path $WorkspaceRoot "marketplace" }
if (-not $SigningKeyPath) { $SigningKeyPath = Join-Path $WorkspaceRoot "_private/sideweave-publisher.key" }
if (-not $MinisignExe) { $MinisignExe = Join-Path $WorkspaceRoot "_private/minisign/minisign.exe" }
foreach ($pathName in @("MarketplacePath", "SigningKeyPath", "MinisignExe")) {
    $value = Get-Variable -Name $pathName -ValueOnly
    if (-not [IO.Path]::IsPathRooted($value)) {
        Set-Variable -Name $pathName -Value (Join-Path $WorkspaceRoot $value)
    }
}
$MarketplacePath = [IO.Path]::GetFullPath($MarketplacePath)
$SigningKeyPath = [IO.Path]::GetFullPath($SigningKeyPath)
if (-not (Test-Path -LiteralPath $MinisignExe -PathType Leaf)) {
    $minisignCommand = Get-Command minisign -ErrorAction SilentlyContinue
    if ($minisignCommand) { $MinisignExe = $minisignCommand.Source }
}

function Read-Required([string]$Prompt) {
    do { $value = (Read-Host $Prompt).Trim() } while ([string]::IsNullOrWhiteSpace($value))
    return $value
}

function Read-YesNo([string]$Prompt) {
    $answer = (Read-Host "$Prompt（输入 y 继续；默认取消）").Trim().ToLowerInvariant()
    return $answer -in @("y", "yes", "是")
}

function Get-OfficialPluginChoices {
    if (-not (Test-Path -LiteralPath $PluginListScript -PathType Leaf)) { throw "插件清单脚本不存在：$PluginListScript" }
    if (-not (Test-Path -LiteralPath $SkinListScript -PathType Leaf)) { throw "皮肤清单脚本不存在：$SkinListScript" }
    $items = @()
    foreach ($entry in @(@{ Script = $PluginListScript; Kind = "plugin" }, @{ Script = $SkinListScript; Kind = "skin" })) {
        $output = & $entry.Script -ListOnly
        if (-not $?) { throw "读取官方$($entry.Kind)清单失败。" }
        $json = (@($output) -join [Environment]::NewLine).Trim()
        if (-not $json) { throw "官方$($entry.Kind)清单为空。" }
        foreach ($item in @($json | ConvertFrom-Json)) {
            $item | Add-Member -NotePropertyName kind -NotePropertyValue $entry.Kind -Force
            $items += $item
        }
    }
    return @($items | Sort-Object kind, id)
}

function Select-PluginChoices([object[]]$Items) {
    if ($Items.Count -eq 0) { throw "没有可选择的插件。" }
    Write-Host ""
    for ($index = 0; $index -lt $Items.Count; $index++) {
        Write-Host ("  {0}. {1} [{2}/{3}] v{4}" -f ($index + 1), $Items[$index].name, $Items[$index].kind, $Items[$index].id, $Items[$index].version)
    }
    do {
        $choice = (Read-Required "选择插件/皮肤编号（如 1,3；输入 all 全选）").ToLowerInvariant()
        if ($choice -eq "all") { return ,$Items }
        $tokens = @($choice -split "[,;\s]+" | Where-Object { $_ })
        $numbers = @()
        $valid = $tokens.Count -gt 0
        foreach ($token in $tokens) {
            $number = 0
            if (-not [int]::TryParse($token, [ref]$number) -or $number -lt 1 -or $number -gt $Items.Count) {
                $valid = $false
                break
            }
            if ($number -notin $numbers) { $numbers += $number }
        }
        if ($valid) {
            $selected = @()
            foreach ($number in $numbers) { $selected += $Items[$number - 1] }
            return ,$selected
        }
        Write-Host "输入无效。请用逗号分隔有效编号，或输入 all。" -ForegroundColor Yellow
    } while ($true)
}

function Invoke-Gh([string[]]$Arguments) {
    & gh @Arguments
    if ($LASTEXITCODE -ne 0) { throw "GitHub CLI 执行失败：gh $($Arguments -join ' ')" }
}

function Invoke-GhCapture([string[]]$Arguments) {
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = & gh @Arguments 2>&1
        $exitCode = $LASTEXITCODE
    }
    finally { $ErrorActionPreference = $previousPreference }
    return [PSCustomObject]@{ ExitCode = $exitCode; Output = @($output) }
}

function Get-GhText([string[]]$Arguments) {
    $result = Invoke-GhCapture $Arguments
    if ($result.ExitCode -ne 0) { throw "GitHub CLI 查询失败：gh $($Arguments -join ' ')`n$(@($result.Output) -join [Environment]::NewLine)" }
    return (@($result.Output) -join [Environment]::NewLine).Trim()
}

function Invoke-GitCapture([string]$Path, [string[]]$Arguments) {
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = & git -C $Path @Arguments 2>&1
        $exitCode = $LASTEXITCODE
    }
    finally { $ErrorActionPreference = $previousPreference }
    return [PSCustomObject]@{ ExitCode = $exitCode; Output = @($output) }
}

function Get-GitText([string]$Path, [string[]]$Arguments) {
    $result = Invoke-GitCapture $Path $Arguments
    if ($result.ExitCode -ne 0) { throw "Git 查询失败：git -C $Path $($Arguments -join ' ')`n$(@($result.Output) -join [Environment]::NewLine)" }
    return (@($result.Output) -join [Environment]::NewLine).Trim()
}

function Invoke-GitChecked([string]$Path, [string[]]$Arguments, [string]$FailureMessage) {
    $result = Invoke-GitCapture $Path $Arguments
    if ($result.ExitCode -ne 0) {
        $details = @($result.Output) -join [Environment]::NewLine
        if ($details) { throw ($FailureMessage + [Environment]::NewLine + $details) }
        throw $FailureMessage
    }
    foreach ($line in $result.Output) { if ($line) { Write-Host $line } }
}

function Get-RemoteTagCommit([string]$Tag) {
    $query = Invoke-GhCapture @("api", "repos/$Repository/git/matching-refs/tags/$Tag")
    if ($query.ExitCode -ne 0) { throw "检查 Git tag '$Tag' 失败：$((@($query.Output) -join [Environment]::NewLine))" }
    try { $refs = @((@($query.Output) -join [Environment]::NewLine) | ConvertFrom-Json) }
    catch { throw "GitHub 返回的 tag '$Tag' 数据无法解析。" }
    $matchingRef = @($refs | Where-Object { [string]$_.ref -eq "refs/tags/$Tag" }) | Select-Object -First 1
    if (-not $matchingRef) { return $null }
    $obj = $matchingRef.object
    for ($depth = 0; $depth -lt 8 -and $obj.type -eq "tag"; $depth++) {
        $tagQuery = Invoke-GhCapture @("api", "repos/$Repository/git/tags/$($obj.sha)")
        if ($tagQuery.ExitCode -ne 0) { throw "无法解析 annotated Git tag '$Tag'：$((@($tagQuery.Output) -join [Environment]::NewLine))" }
        try { $tagObject = (@($tagQuery.Output) -join [Environment]::NewLine) | ConvertFrom-Json }
        catch { throw "GitHub 返回的 annotated tag '$Tag' 数据无法解析。" }
        $obj = $tagObject.object
    }
    if ($obj.type -eq "tag") { throw "Git tag '$Tag' 的嵌套层数超出支持范围。" }
    if ($obj.type -ne "commit") { throw "Git tag '$Tag' 没有指向 commit。" }
    return [string]$obj.sha
}

function Get-ReleaseInfo([string]$Tag) {
    $query = Invoke-GhCapture @("release", "view", $Tag, "--repo", $Repository, "--json", "tagName,isDraft")
    if ($query.ExitCode -ne 0) {
        $errorText = @($query.Output) -join [Environment]::NewLine
        if ($errorText -match "(?i)(\b404\b|release .*not found|not found)") { return $null }
        throw "查询 GitHub Release '$Tag' 失败：$errorText"
    }
    try { return ((@($query.Output) -join [Environment]::NewLine) | ConvertFrom-Json) }
    catch { throw "GitHub Release '$Tag' 响应无法解析。" }
}

function Assert-TagTargetsCommit([string]$Tag, [string]$Commit, $ReleaseInfo) {
    $tagCommit = Get-RemoteTagCommit $Tag
    if (-not $tagCommit) {
        if ($ReleaseInfo) { throw "Release '$Tag' 已存在，但无法验证它对应的 Git tag。" }
        return
    }
    if ([string]$tagCommit -ne $Commit) {
        throw "Git tag '$Tag' 已指向其他源码提交。请递增插件版本后再发布，脚本不会覆盖不同源码的 tag。"
    }
}

function Assert-PluginVersionDoesNotRegress([string]$PluginId, [string]$Candidate, [string[]]$ExistingVersions) {
    $candidateCoreText = ($Candidate -split "[-+]")[0]
    try { $candidateCore = [version]$candidateCoreText }
    catch { throw "无法解析 $PluginId 的 SemVer 版本：$Candidate" }
    $candidatePrerelease = if ($Candidate -match '^[0-9]+\.[0-9]+\.[0-9]+-([^+]+)') { $Matches[1] } else { "" }

    foreach ($existing in $ExistingVersions) {
        if ($existing -eq $Candidate) { continue }
        $existingCoreText = ($existing -split "[-+]")[0]
        try { $existingCore = [version]$existingCoreText }
        catch { throw "Release tag 中的版本号无法解析：$PluginId@$existing" }
        if ($candidateCore -lt $existingCore) {
            throw "$PluginId 发布版本 $Candidate 低于现有 Release $existing；请先更新版本号，脚本不会删除较新的 Release。"
        }
        if ($candidateCore -gt $existingCore) { continue }

        $existingPrerelease = if ($existing -match '^[0-9]+\.[0-9]+\.[0-9]+-([^+]+)') { $Matches[1] } else { "" }
        if (-not $candidatePrerelease -and $existingPrerelease) { continue }
        if ($candidatePrerelease -and -not $existingPrerelease) {
            throw "$PluginId 发布版本 $Candidate 是预发布版，低于现有稳定 Release $existing。"
        }
        if ($candidatePrerelease -and $existingPrerelease -and $candidatePrerelease -ne $existingPrerelease) {
            throw "$PluginId 版本 $Candidate 与现有预发布版 $existing 的优先级无法安全判断；请递增 SemVer 主/次/补丁段。"
        }
        if ($candidate -ne $existing) {
            throw "$PluginId 版本 $Candidate 与现有 Release $existing 的 SemVer 优先级相同；请使用新的 SemVer 版本。"
        }
    }
}

function Get-ReleaseCleanupPlan([object[]]$Packages) {
    $query = Invoke-GhCapture @("release", "list", "--repo", $Repository, "--limit", "1000", "--json", "tagName,isDraft")
    if ($query.ExitCode -ne 0) { throw "无法列出 GitHub Releases：$((@($query.Output) -join [Environment]::NewLine))" }
    try { $releases = @((@($query.Output) -join [Environment]::NewLine) | ConvertFrom-Json) }
    catch { throw "GitHub Releases 清单无法解析。" }
    if ($releases.Count -ge 1000) { throw "GitHub Release 数量达到 1000 条上限，无法安全确认所有历史版本；请先手动检查仓库。" }

    $cleanup = @()
    foreach ($plan in $Packages) {
        $tagPrefix = if ([string]$plan.Kind -eq "skin") { "skin" } else { "plugin" }
        $tagPattern = '^' + $tagPrefix + '-' + [regex]::Escape([string]$plan.Id) + '-v(?<version>(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?)$'
        $releaseByTag = @{}
        $existingVersions = @()
        foreach ($release in $releases) {
            $tag = [string]$release.tagName
            if ($tag -match $tagPattern) {
                $releaseByTag[$tag] = [PSCustomObject]@{ Tag = $tag; Version = [string]$Matches.version; IsDraft = [bool]$release.isDraft }
                $existingVersions += [string]$Matches.version
            }
        }

        $tagQuery = Invoke-GitCapture $RepoRoot @("ls-remote", "--tags", "origin", "refs/tags/$tagPrefix-$($plan.Id)-v*")
        if ($tagQuery.ExitCode -ne 0) { throw "无法列出 $($plan.Id) 的 Git tags：$((@($tagQuery.Output) -join [Environment]::NewLine))" }
        $pluginTags = @(
            foreach ($line in $tagQuery.Output) {
                if ([string]$line -notmatch '^\S+\s+refs/tags/(?<tag>.+)$') { continue }
                $tag = [string]$Matches.tag
                if ($tag.EndsWith("^{}", [StringComparison]::Ordinal)) { continue }
                if ($tag -match $tagPattern) {
                    [PSCustomObject]@{
                        Tag = $tag
                        Version = [string]$Matches.version
                        IsDraft = if ($releaseByTag.ContainsKey($tag)) { [bool]$releaseByTag[$tag].IsDraft } else { $false }
                        HasRelease = $releaseByTag.ContainsKey($tag)
                    }
                }
            }
        )
        foreach ($tag in $pluginTags) {
            if ($tag.Version -notin $existingVersions) { $existingVersions += [string]$tag.Version }
        }
        Assert-PluginVersionDoesNotRegress -PluginId ([string]$plan.Id) -Candidate ([string]$plan.Version) -ExistingVersions $existingVersions

        $seenTags = @{}
        foreach ($oldTag in $pluginTags) {
            if ($oldTag.Tag -eq [string]$plan.Package.Tag) { continue }
            $seenTags[$oldTag.Tag] = $true
            $cleanup += [PSCustomObject]@{
                Id = [string]$plan.Id
                Kind = [string]$plan.Kind
                Version = [string]$oldTag.Version
                Tag = [string]$oldTag.Tag
                IsDraft = [bool]$oldTag.IsDraft
                HasRelease = [bool]$oldTag.HasRelease
            }
        }
        foreach ($oldRelease in $releaseByTag.Values) {
            if ($oldRelease.Tag -eq [string]$plan.Package.Tag -or $seenTags.ContainsKey($oldRelease.Tag)) { continue }
            $cleanup += [PSCustomObject]@{
                Id = [string]$plan.Id
                Kind = [string]$plan.Kind
                Version = [string]$oldRelease.Version
                Tag = [string]$oldRelease.Tag
                IsDraft = [bool]$oldRelease.IsDraft
                HasRelease = $true
            }
        }
    }
    return $cleanup | Sort-Object Id, Version
}

function Remove-OldPluginReleases([object[]]$Releases) {
    foreach ($release in $Releases) {
        if ($release.HasRelease) {
            Write-Host "删除旧版 Release 和 tag：$($release.Tag)" -ForegroundColor Yellow
            Invoke-Gh @("release", "delete", $release.Tag, "--repo", $Repository, "--cleanup-tag", "--yes")
        }
        else {
            Write-Host "删除无 Release 的旧 tag：$($release.Tag)" -ForegroundColor Yellow
            Invoke-Gh @("api", "--method", "DELETE", "repos/$Repository/git/refs/tags/$($release.Tag)")
        }
    }
    if ($Releases.Count -gt 0) {
        Write-Host "旧版本 Release 和关联 tag 已清理。" -ForegroundColor Green
    }
}

function Assert-LocalSourceMatchesGitHub {
    $root = Invoke-GitCapture $RepoRoot @("rev-parse", "--show-toplevel")
    if ($root.ExitCode -ne 0) { throw "插件源码目录不是可用的 Git 仓库：$RepoRoot" }
    $actualRoot = [IO.Path]::GetFullPath((@($root.Output) -join "").Trim()).TrimEnd('\', '/')
    $expectedRoot = [IO.Path]::GetFullPath($RepoRoot).TrimEnd('\', '/')
    if (-not [StringComparer]::OrdinalIgnoreCase.Equals($actualRoot, $expectedRoot)) { throw "脚本必须从 SideWeave-Plugins 仓库运行。" }

    $status = Invoke-GitCapture $RepoRoot @("status", "--porcelain")
    if ($status.ExitCode -ne 0) { throw "无法读取插件仓库 Git 状态。" }
    if (@($status.Output | Where-Object { $_ }).Count -gt 0) {
        throw "插件仓库有未提交改动。请先提交并推送，再发布，确保 Release tag 对应本次构建源码。"
    }
    $origin = Get-GitText $RepoRoot @("remote", "get-url", "origin")
    if ($origin -notmatch '(?i)github\.com[:/]([^/]+)/([^/]+)$') { throw "无法从插件仓库 origin 解析 GitHub 仓库：$origin" }
    $sourceRepository = "$($Matches[1])/$($Matches[2] -replace '\.git$', '')"
    if (-not [StringComparer]::OrdinalIgnoreCase.Equals($sourceRepository, $Repository)) {
        throw "插件仓库 origin 指向 '$sourceRepository'，发布目标是 '$Repository'。请检查 Repository 参数或仓库远端。"
    }
    $branch = Get-GitText $RepoRoot @("branch", "--show-current")
    if (-not $branch) { throw "无法读取插件仓库当前 Git 分支。" }
    $defaultBranch = Get-GhText @("repo", "view", $Repository, "--json", "defaultBranchRef", "--jq", ".defaultBranchRef.name")
    if ($branch -ne $defaultBranch) { throw "当前分支 '$branch' 不是 GitHub 默认分支 '$defaultBranch'。请切换后再发布。" }

    $localCommit = Get-GitText $RepoRoot @("rev-parse", "HEAD")
    $remoteCommit = Get-GhText @("api", "repos/$Repository/branches/$defaultBranch", "--jq", ".commit.sha")
    if (-not $localCommit -or $localCommit -ne $remoteCommit) {
        throw "本地提交尚未与 GitHub 默认分支同步。请先 push 最新提交，再发布。"
    }
    return $localCommit
}

function Read-PackageManifestFromZip([string]$ArchivePath) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::OpenRead($ArchivePath)
    try {
        $entry = $archive.GetEntry("manifest.json")
        if (-not $entry) { throw "包缺少根目录 manifest.json：$ArchivePath" }
        $stream = $entry.Open()
        try {
            $reader = [System.IO.StreamReader]::new($stream, [System.Text.Encoding]::UTF8)
            try { $text = $reader.ReadToEnd() }
            finally { $reader.Dispose() }
        }
        finally { if ($stream) { $stream.Dispose() } }
    }
    finally { $archive.Dispose() }
    try { return $text | ConvertFrom-Json }
    catch { throw "包内 manifest.json 无法解析：$ArchivePath" }
}

function Get-PackageAssets([string]$PluginId, [string]$PluginVersion, [string]$Kind = "plugin") {
    $tagPrefix = if ($Kind -eq "skin") { "skin" } else { "plugin" }
    $tag = "$tagPrefix-$PluginId-v$PluginVersion"
    $releaseDir = Join-Path $OutputRoot $tag
    $archivePath = Join-Path $releaseDir "$PluginId-$PluginVersion.zip"
    $signaturePath = "$archivePath.minisig"
    $checksumPath = "$archivePath.sha256"
    foreach ($path in @($archivePath, $signaturePath, $checksumPath)) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "签名发布资产不存在：$path。先运行本地构建，或检查 -SkipBuild 的版本目录。" }
    }

    $manifest = Read-PackageManifestFromZip $archivePath
    if ([string]$manifest.id -ne $PluginId -or [string]$manifest.version -ne $PluginVersion) {
        throw "包内 manifest 与选择的$Kind版本不匹配：$PluginId@$PluginVersion。"
    }
    $expectedType = if ($Kind -eq "skin") { "skin" } else { "native" }
    if ([string]$manifest.type -ne $expectedType) { throw "包内 manifest type 与选择类型不匹配：$PluginId。" }
    $publisher = Get-PublisherKey -PublisherId ([string]$manifest.publisher) -KeyId ([string]$manifest.key_id)
    $checksumLine = (Get-Content -LiteralPath $checksumPath -Raw -Encoding UTF8).Trim()
    $actualHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($checksumLine -notmatch "^$actualHash\s+$([regex]::Escape([IO.Path]::GetFileName($archivePath)))$") {
        throw "SHA256 清单与包文件不匹配：$checksumPath"
    }
    & $MinisignExe -V -m $archivePath -x $signaturePath -P ([string]$publisher.Key.public_key)
    if ($LASTEXITCODE -ne 0) { throw "Minisign 签名验证失败：$archivePath" }
    return [PSCustomObject]@{
        Tag = $tag
        Assets = @($archivePath, $signaturePath, $checksumPath)
        ReleaseDir = $releaseDir
        Manifest = $manifest
    }
}

function Get-PublisherKey([string]$PublisherId, [string]$KeyId) {
    $publishersFile = Join-Path $MarketplacePath "publishers.json"
    if (-not (Test-Path -LiteralPath $publishersFile -PathType Leaf)) { throw "Marketplace publishers.json 不存在：$publishersFile" }
    try { $directory = Get-Content -LiteralPath $publishersFile -Raw -Encoding UTF8 | ConvertFrom-Json }
    catch { throw "publishers.json 无法解析：$($_.Exception.Message)" }
    $publisher = @($directory.publishers | Where-Object { [string]$_.id -eq $PublisherId }) | Select-Object -First 1
    if (-not $publisher) { throw "publishers.json 中不存在 publisher：$PublisherId" }
    $key = @($publisher.keys | Where-Object { [string]$_.id -eq $KeyId }) | Select-Object -First 1
    if (-not $key -or [string]$key.algorithm -ne "minisign" -or [string]$key.status -ne "active") {
        throw "发布包使用的 $PublisherId/$KeyId 不是 publishers.json 中的 active Minisign 密钥。"
    }
    return [PSCustomObject]@{ Directory = $directory; Publisher = $publisher; Key = $key }
}

function Get-MarketplaceContext([object[]]$Packages) {
    foreach ($fileName in @("marketplace.json", "publishers.json")) {
        $path = Join-Path $MarketplacePath $fileName
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Marketplace 文件不存在：$path" }
    }
    $root = Invoke-GitCapture $MarketplacePath @("rev-parse", "--show-toplevel")
    if ($root.ExitCode -ne 0) { throw "MarketplacePath 不是 Git 仓库：$MarketplacePath" }
    $actualRoot = [IO.Path]::GetFullPath((@($root.Output) -join "").Trim()).TrimEnd('\', '/')
    $expectedRoot = [IO.Path]::GetFullPath($MarketplacePath).TrimEnd('\', '/')
    if (-not [StringComparer]::OrdinalIgnoreCase.Equals($actualRoot, $expectedRoot)) { throw "MarketplacePath 必须指向仓库根目录：$expectedRoot" }
    $status = Invoke-GitCapture $actualRoot @("status", "--porcelain")
    if ($status.ExitCode -ne 0) { throw "无法读取 Marketplace Git 状态。" }
    if (@($status.Output | Where-Object { $_ }).Count -gt 0) { throw "Marketplace 仓库有未提交改动；请先提交或清理，再同步发布目录。" }
    $remote = Invoke-GitCapture $actualRoot @("remote", "get-url", "origin")
    if ($remote.ExitCode -ne 0) { throw "Marketplace 仓库没有 origin 远端。" }
    $remoteUrl = (@($remote.Output) -join "").Trim().TrimEnd('/')
    if ($remoteUrl -notmatch '(?i)github\.com[:/]([^/]+)/([^/]+)$') { throw "无法从 Marketplace origin 解析 GitHub 仓库：$remoteUrl" }
    $marketplaceRepository = "$($Matches[1])/$($Matches[2] -replace '\.git$', '')"
    $branch = Get-GitText $actualRoot @("branch", "--show-current")
    $defaultBranch = Get-GhText @("repo", "view", $marketplaceRepository, "--json", "defaultBranchRef", "--jq", ".defaultBranchRef.name")
    if (-not $branch -or $branch -ne $defaultBranch) { throw "Marketplace 当前分支 '$branch' 不是 GitHub 默认分支 '$defaultBranch'。" }
    $localCommit = Get-GitText $actualRoot @("rev-parse", "HEAD")
    $remoteCommit = Get-GhText @("api", "repos/$marketplaceRepository/branches/$defaultBranch", "--jq", ".commit.sha")
    if ($localCommit -ne $remoteCommit) { throw "Marketplace 本地提交尚未与 GitHub 默认分支同步；未修改目录文件。" }
    try {
        $marketplace = Get-Content -LiteralPath (Join-Path $actualRoot "marketplace.json") -Raw -Encoding UTF8 | ConvertFrom-Json
        $publisherDirectory = Get-Content -LiteralPath (Join-Path $actualRoot "publishers.json") -Raw -Encoding UTF8 | ConvertFrom-Json
    }
    catch { throw "无法解析 marketplace.json 或 publishers.json：$($_.Exception.Message)" }
    Assert-MarketplaceDocument $marketplace $publisherDirectory
    foreach ($plan in $Packages) {
        $manifest = $plan.Package.Manifest
        $collectionName = if ([string]$plan.Kind -eq "skin") { "skins" } else { "plugins" }
        $collection = @($marketplace.PSObject.Properties[$collectionName].Value)
        $item = @($collection | Where-Object { [string]$_.id -eq [string]$plan.Id }) | Select-Object -First 1
        if (-not $item) { continue }
        if ([string]$item.publisher -and [string]$item.publisher -ne [string]$manifest.publisher) { throw "marketplace.json 中 $($plan.Id) 的 publisher 与包内 manifest 不一致。" }
        if ([string]$item.type -and [string]$item.type -ne [string]$manifest.type) { throw "marketplace.json 中 $($plan.Id) 的 type 与包内 manifest 不一致。" }
        $currentVersion = [string]$item.latest
        if ($currentVersion -and $currentVersion -ne [string]$plan.Version) {
            try { $isOlder = [version](($plan.Version -split "[-+]")[0]) -lt [version](($currentVersion -split "[-+]")[0]) }
            catch { throw "无法比较 Marketplace 版本号：$currentVersion 和 $($plan.Version)" }
            if ($isOlder) { throw "发布版本 $($plan.Version) 低于 Marketplace 当前 latest $currentVersion；拒绝降级目录。" }
        }
    }
    return [PSCustomObject]@{ Root = $actualRoot; Repository = $marketplaceRepository; Branch = $branch }
}

function Assert-MarketplaceDocument($Marketplace, $PublisherDirectory) {
    if ([int]$Marketplace.schema_version -ne 1) { throw "marketplace.json 的 schema_version 必须是 1。" }
    if ([int]$PublisherDirectory.schema_version -ne 1) { throw "publishers.json 的 schema_version 必须是 1。" }
    foreach ($collectionName in @("plugins", "skins")) {
        $seenIds = @{}
        foreach ($item in @($Marketplace.$collectionName)) {
            $itemId = [string]$item.id
            if ($itemId -notmatch '^[a-z0-9][a-z0-9_-]*$') { throw "市场条目 ID 格式无效：$itemId" }
            if ($seenIds.ContainsKey($itemId)) { throw "marketplace.json 中有重复 ID：$itemId" }
            $seenIds[$itemId] = $true
            if ($collectionName -eq "plugins" -and [string]$item.type -notin @("web", "native")) { throw "插件条目 $itemId 的 type 必须是 web 或 native。" }
            if ($collectionName -eq "skins" -and [string]$item.type -ne "skin") { throw "皮肤条目 $itemId 的 type 必须是 skin。" }
            $publisher = @($PublisherDirectory.publishers | Where-Object { [string]$_.id -eq [string]$item.publisher }) | Select-Object -First 1
            if (-not $publisher) { throw "市场条目 $itemId 引用了未知 publisher：$($item.publisher)" }
            $versions = @($item.versions)
            if ($versions.Count -eq 0 -or @($versions | Where-Object { [string]$_.version -eq [string]$item.latest }).Count -eq 0) {
                throw "市场条目 $itemId 的 latest 必须对应 versions 中的一个版本。"
            }
            foreach ($entry in $versions) {
                if (@($publisher.keys | Where-Object { [string]$_.id -eq [string]$entry.key_id }).Count -eq 0) { throw "市场条目 $itemId@$($entry.version) 引用了未知 key_id：$($entry.key_id)" }
                if (-not $entry.release -or -not $entry.release.tag -or -not $entry.release.asset) { throw "市场条目 $itemId@$($entry.version) 缺少 tag 或 asset。" }
                foreach ($urlProperty in @("url", "signature_url", "checksum_url")) {
                    if ([string]$entry.release.$urlProperty -notmatch '^https://[^/]+/.+') { throw "市场条目 $itemId@$($entry.version) 的 $urlProperty 必须是 HTTPS 下载地址。" }
                }
            }
        }
    }
}

function Set-ObjectProperty($Object, [string]$Name, $Value) {
    if ($null -eq $Object.PSObject.Properties[$Name]) { $Object | Add-Member -MemberType NoteProperty -Name $Name -Value $Value }
    else { $Object.$Name = $Value }
}

function Get-ReleaseAssetUrls([string]$Tag, [string[]]$AssetPaths) {
    $query = Invoke-GhCapture @("api", "repos/$Repository/releases/tags/$Tag")
    if ($query.ExitCode -ne 0) { throw "无法读取已发布的 Release '$Tag'：$((@($query.Output) -join [Environment]::NewLine))" }
    try { $release = (@($query.Output) -join [Environment]::NewLine) | ConvertFrom-Json }
    catch { throw "GitHub Release '$Tag' 响应无法解析。" }
    $remoteAssets = @{}
    foreach ($asset in @($release.assets)) { $remoteAssets[[string]$asset.name] = [string]$asset.browser_download_url }
    $urls = @{}
    foreach ($assetPath in $AssetPaths) {
        $name = [IO.Path]::GetFileName($assetPath)
        if (-not $remoteAssets.ContainsKey($name) -or $remoteAssets[$name] -notmatch '^https://') { throw "Release '$Tag' 缺少 HTTPS 资产 '$name'。" }
        $urls[$name] = $remoteAssets[$name]
    }
    return $urls
}

function Update-MarketplaceAfterPublish([object[]]$PublishedPackages, $Context) {
    $marketplaceFile = Join-Path $Context.Root "marketplace.json"
    $publishersFile = Join-Path $Context.Root "publishers.json"
    try {
        $marketplace = Get-Content -LiteralPath $marketplaceFile -Raw -Encoding UTF8 | ConvertFrom-Json
        $publisherDirectory = Get-Content -LiteralPath $publishersFile -Raw -Encoding UTF8 | ConvertFrom-Json
    }
    catch { throw "无法解析 marketplace.json 或 publishers.json：$($_.Exception.Message)" }
    Assert-MarketplaceDocument $marketplace $publisherDirectory

    foreach ($plan in $PublishedPackages) {
        $manifest = $plan.Package.Manifest
        $id = [string]$plan.Id
        $version = [string]$plan.Version
        if ([string]$manifest.publisher -ne "sideweave") { throw "官方插件 $id 的 publisher 必须是 sideweave。" }
        $publisher = Get-PublisherKey -PublisherId ([string]$manifest.publisher) -KeyId ([string]$manifest.key_id)
        $assetUrls = Get-ReleaseAssetUrls -Tag $plan.Package.Tag -AssetPaths $plan.Package.Assets
        $archiveName = [IO.Path]::GetFileName($plan.Package.Assets[0])
        $signatureName = [IO.Path]::GetFileName($plan.Package.Assets[1])
        $checksumName = [IO.Path]::GetFileName($plan.Package.Assets[2])
        $versionEntry = [PSCustomObject][ordered]@{
            version = $version
            key_id = [string]$manifest.key_id
            release = [PSCustomObject][ordered]@{
                tag = [string]$plan.Package.Tag
                asset = $archiveName
                url = $assetUrls[$archiveName]
                signature_url = $assetUrls[$signatureName]
                checksum_url = $assetUrls[$checksumName]
            }
        }
        $collectionName = if ([string]$plan.Kind -eq "skin") { "skins" } else { "plugins" }
        $items = @($marketplace.PSObject.Properties[$collectionName].Value)
        $item = @($items | Where-Object { [string]$_.id -eq $id }) | Select-Object -First 1
        if ($item) {
            if ([string]$item.publisher -and [string]$item.publisher -ne [string]$manifest.publisher) { throw "marketplace.json 中 $id 的 publisher 与包内 manifest 不一致。" }
            if ([string]$item.type -and [string]$item.type -ne [string]$manifest.type) { throw "marketplace.json 中 $id 的 type 与包内 manifest 不一致。" }
            $currentVersion = [string]$item.latest
            if ($currentVersion -and $currentVersion -ne $version) {
                $currentCore = ($currentVersion -split "[-+]")[0]
                $newCore = ($version -split "[-+]")[0]
                try { $isOlder = [version]$newCore -lt [version]$currentCore }
                catch { throw "无法比较 Marketplace 版本号：$currentVersion 和 $version" }
                if ($isOlder) { throw "发布版本 $version 低于 Marketplace 当前 latest $currentVersion；拒绝降级目录。" }
            }
        }
        else {
            $capabilities = if ($null -ne $manifest.capabilities) { @($manifest.capabilities) } elseif ($null -ne $manifest.permissions) { @($manifest.permissions) } else { @() }
            $item = [PSCustomObject][ordered]@{
                id = $id
                name = [string]$manifest.name
                description = [string]$manifest.description
                author = [string]$manifest.author
                publisher = [string]$manifest.publisher
                type = [string]$manifest.type
                repository = "https://github.com/$Repository"
                verified = $true
                latest = $version
                capabilities = @($capabilities)
                versions = @()
            }
            $items += $item
        }
        $capabilities = if ($null -ne $manifest.capabilities) { @($manifest.capabilities) } elseif ($null -ne $manifest.permissions) { @($manifest.permissions) } else { @() }
        Set-ObjectProperty $item "name" ([string]$manifest.name)
        Set-ObjectProperty $item "description" ([string]$manifest.description)
        Set-ObjectProperty $item "author" ([string]$manifest.author)
        Set-ObjectProperty $item "publisher" ([string]$manifest.publisher)
        Set-ObjectProperty $item "type" ([string]$manifest.type)
        Set-ObjectProperty $item "repository" "https://github.com/$Repository"
        Set-ObjectProperty $item "verified" $true
        Set-ObjectProperty $item "capabilities" @($capabilities)
        Set-ObjectProperty $item "latest" $version
        Set-ObjectProperty $item "versions" @($versionEntry)
        Set-ObjectProperty $marketplace $collectionName @($items)
        Write-Host "已准备目录条目：$($plan.Kind)/$id v$version" -ForegroundColor Cyan
    }
    Set-ObjectProperty $marketplace "updated_at" ([DateTime]::UtcNow.ToString("yyyy-MM-dd'T'HH:mm:ss'Z'"))
    Assert-MarketplaceDocument $marketplace $publisherDirectory

    $originalJson = [IO.File]::ReadAllText($marketplaceFile)
    $updatedJson = ConvertTo-Json -InputObject $marketplace -Depth 100
    $temporaryFile = Join-Path $Context.Root (".marketplace.json.{0}.tmp" -f [Guid]::NewGuid().ToString("N"))
    try {
        [IO.File]::WriteAllText($temporaryFile, $updatedJson + [Environment]::NewLine, [System.Text.UTF8Encoding]::new($false))
        Move-Item -LiteralPath $temporaryFile -Destination $marketplaceFile -Force
        $written = Get-Content -LiteralPath $marketplaceFile -Raw -Encoding UTF8 | ConvertFrom-Json
        Assert-MarketplaceDocument $written $publisherDirectory
    }
    catch {
        Remove-Item -LiteralPath $temporaryFile -Force -ErrorAction SilentlyContinue
        [IO.File]::WriteAllText($marketplaceFile, $originalJson, [System.Text.UTF8Encoding]::new($false))
        throw "Marketplace 目录更新校验失败，已恢复原文件：$($_.Exception.Message)"
    }

    Invoke-GitChecked $Context.Root @("add", "--", "marketplace.json") "已更新 marketplace.json，但 git add 失败。"
    $stagedDiff = Invoke-GitCapture $Context.Root @("diff", "--cached", "--quiet")
    if ($stagedDiff.ExitCode -eq 0) {
        Write-Host "marketplace.json 已包含这些版本和资产 URL，没有需要提交的新改动。" -ForegroundColor Green
        return
    }
    if ($stagedDiff.ExitCode -ne 1) { throw "已更新并暂存 marketplace.json，但无法检查暂存差异。" }
    $summary = @($PublishedPackages | ForEach-Object { "$($_.Id)@$($_.Version)" }) -join ", "
    $message = "Update marketplace packages: $summary"
    Invoke-GitChecked $Context.Root @("commit", "-m", $message) "marketplace.json 已暂存，但自动提交失败。"
    Invoke-GitChecked $Context.Root @("push", "origin", $Context.Branch) "marketplace.json 已提交到本地，但推送失败；请稍后手动推送。"
    $localCommit = Get-GitText $Context.Root @("rev-parse", "HEAD")
    $remoteCommit = Get-GhText @("api", "repos/$($Context.Repository)/branches/$($Context.Branch)", "--jq", ".commit.sha")
    if ($localCommit -ne $remoteCommit) { throw "marketplace.json 提交已推送，但 GitHub 默认分支暂未同步到该提交。" }
    Write-Host "marketplace.json 已更新、校验、提交并推送：$message" -ForegroundColor Green
}

function Publish-GitHubRelease([object]$Plan, [string]$SourceCommit) {
    $tag = [string]$Plan.Package.Tag
    $releaseInfo = Get-ReleaseInfo $tag
    Assert-TagTargetsCommit -Tag $tag -Commit $SourceCommit -ReleaseInfo $releaseInfo
    $kind = if ([string]$Plan.Kind -eq "skin") { "skin" } else { "plugin" }
    $title = "SideWeave $kind $($Plan.Id) v$($Plan.Version)"
    $notes = if ($Notes) { $Notes } else { "SideWeave official $kind $($Plan.Id) v$($Plan.Version)." }
    if ($releaseInfo) {
        $args = @("release", "upload", $tag) + @($Plan.Package.Assets) + @("--repo", $Repository, "--clobber")
        Invoke-Gh $args
        Invoke-Gh @("release", "edit", $tag, "--repo", $Repository, "--title", $title, "--draft=false", "--latest=false")
    }
    else {
        $args = @("release", "create", $tag) + @($Plan.Package.Assets) + @(
            "--repo", $Repository, "--title", $title, "--notes", $notes, "--target", $SourceCommit, "--latest=false"
        )
        Invoke-Gh $args
    }
    Write-Host "已上传 $tag" -ForegroundColor Green
}

if (-not (Test-Path -LiteralPath $RepoRoot -PathType Container)) { throw "SideWeave-Plugins 仓库目录不存在：$RepoRoot" }
if (-not (Test-Path -LiteralPath $ReleaseBuildScript -PathType Leaf)) { throw "签名构建脚本不存在：$ReleaseBuildScript" }
if ($Repository -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$') { throw "Repository 格式应为 owner/repository。" }

$interactive = -not $LocalOnly -and -not $PreviewOnly -and [string]::IsNullOrWhiteSpace($Id)
if ($interactive) {
    Write-Host ""
    Write-Host "SideWeave-Plugins 打包与发布向导" -ForegroundColor Cyan
        Write-Host "  1. 本地构建并签名插件/皮肤包"
        Write-Host "  2. 发布插件/皮肤到 GitHub Release"
    $mode = Read-Required "选择 1 或 2"
    if ($mode -eq "1") { $LocalOnly = $true }
    elseif ($mode -ne "2") { throw "请输入 1 或 2。" }
}

$pluginChoices = @(Get-OfficialPluginChoices)
$selectedPlugins = @()
if ([string]::IsNullOrWhiteSpace($Id)) {
    $selectedPlugins = @(Select-PluginChoices $pluginChoices)
}
else {
    $requestedIds = @($Id -split "[,;\s]+" | Where-Object { $_ } | Select-Object -Unique)
    if ($requestedIds.Count -eq 1 -and $requestedIds[0].ToLowerInvariant() -eq "all") { $selectedPlugins = $pluginChoices }
    else {
        foreach ($requestedId in $requestedIds) {
            $match = @($pluginChoices | Where-Object { [string]$_.id -eq $requestedId }) | Select-Object -First 1
            if (-not $match) { throw "官方插件/皮肤清单中不存在：$requestedId" }
            $selectedPlugins += $match
        }
    }
}
if ($selectedPlugins.Count -eq 0) { throw "没有选择官方插件或皮肤。" }
if ($Version -and $selectedPlugins.Count -gt 1) { throw "批量发布时请在各 manifest.json 中分别设置版本；-Version 只支持单个项目。" }
foreach ($plugin in $selectedPlugins) {
    $pluginVersion = if ($Version) { $Version } else { [string]$plugin.version }
    if ($pluginVersion -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$') { throw "$($plugin.kind) $($plugin.id) 的 SemVer 版本无效：$pluginVersion" }
    if ($pluginVersion -ne [string]$plugin.version) { throw "-Version 必须与 $($plugin.id) 的 manifest.json 版本一致（$($plugin.version)）。请先更新并提交 manifest 版本。" }
}

if (-not $LocalOnly -and -not (Get-Command gh -ErrorAction SilentlyContinue)) { throw "未找到 GitHub CLI。请安装 gh 并运行 gh auth login。" }
if (-not (Test-Path -LiteralPath $MinisignExe -PathType Leaf)) { throw "未找到 Minisign。请安装 minisign 或使用 -MinisignExe 指定路径。" }
if (-not (Test-Path -LiteralPath $SigningKeyPath -PathType Leaf)) { throw "未找到 SideWeave publisher 私钥：$SigningKeyPath" }

if ($LocalOnly) {
    Write-Host ""
    Write-Host "本地打包模式：允许未提交改动，不检查 GitHub 登录，也不会上传。" -ForegroundColor Cyan
}
else {
    Write-Host ""
    Write-Host "检查 GitHub 登录和插件源码状态..." -ForegroundColor Cyan
    Invoke-Gh @("auth", "status", "--hostname", "github.com")
    $sourceCommit = Assert-LocalSourceMatchesGitHub
}

$builtPackages = @()
foreach ($plugin in $selectedPlugins) {
    $pluginId = [string]$plugin.id
    $pluginVersion = if ($Version) { $Version } else { [string]$plugin.version }
    if (-not $SkipBuild) {
        Write-Host ""
        Write-Host "开始构建并签名 $($plugin.kind)/$pluginId v$pluginVersion..." -ForegroundColor Cyan
        $buildArgs = @{
            Id = $pluginId
            Kind = [string]$plugin.kind
            Version = $pluginVersion
            SigningKeyPath = $SigningKeyPath
            MarketplacePath = $MarketplacePath
            MinisignExe = $MinisignExe
            OutputPath = $OutputRoot
        }
        & $ReleaseBuildScript @buildArgs
        if ($LASTEXITCODE -ne 0) { throw "本地包构建或签名失败：$pluginId" }
    }
    $kind = [string]$plugin.kind
    $package = Get-PackageAssets -PluginId $pluginId -PluginVersion $pluginVersion -Kind $kind
    $builtPackages += [PSCustomObject]@{ Id = $pluginId; Version = $pluginVersion; Kind = $kind; Package = $package }
}

if ($LocalOnly) {
    Write-Host ""
    Write-Host "所选官方插件本地打包、签名和校验完成；文件未上传：" -ForegroundColor Green
    foreach ($plan in $builtPackages) {
        Write-Host "  $($plan.Kind)/$($plan.Id) v$($plan.Version)"
        foreach ($asset in $plan.Package.Assets) { Write-Host "    $asset" }
    }
    return
}

Write-Host ""
Write-Host "发布目标：$Repository" -ForegroundColor Cyan
Write-Host "源码提交：$sourceCommit"
foreach ($plan in $builtPackages) {
    $releaseInfo = Get-ReleaseInfo $plan.Package.Tag
    Assert-TagTargetsCommit -Tag $plan.Package.Tag -Commit $sourceCommit -ReleaseInfo $releaseInfo
    Write-Host "Release：$($plan.Package.Tag)"
    if ($releaseInfo) { Write-Host "  操作：更新现有 Release，同名资产会被替换。" }
    else { Write-Host "  操作：创建新 Release。" }
    foreach ($asset in $plan.Package.Assets) { Write-Host "  $asset" }
}

$releaseCleanupPlan = @(Get-ReleaseCleanupPlan -Packages $builtPackages)
if ($releaseCleanupPlan.Count -gt 0) {
    Write-Host ""
    Write-Host "同步 Marketplace 后将删除以下旧版本 Release 和 tag：" -ForegroundColor Yellow
    foreach ($oldRelease in $releaseCleanupPlan) {
        Write-Host "  $($oldRelease.Kind)/$($oldRelease.Id) v$($oldRelease.Version) [$($oldRelease.Tag)]"
    }
}
else {
    Write-Host ""
    Write-Host "没有需要清理的旧版插件/皮肤 Release。"
}

if ($PreviewOnly) {
    Write-Host "PreviewOnly：未上传文件、未更新 Marketplace，也未删除旧版本。" -ForegroundColor Yellow
    return
}
if (-not $SkipMarketplaceUpdate) {
    Write-Host "检查 Marketplace 仓库状态和目录..." -ForegroundColor Cyan
    $marketplaceContext = Get-MarketplaceContext -Packages $builtPackages
}
$confirmationPrompt = if ($SkipMarketplaceUpdate) {
    "确认只上传以上 $($builtPackages.Count) 个包，不更新 Marketplace，也不清理旧版 Release？"
}
elseif ($releaseCleanupPlan.Count -gt 0) {
    "确认上传新版本、更新 Marketplace 到 latest，随后删除列出的 $($releaseCleanupPlan.Count) 个旧版 Release 和 tag？"
}
else {
    "确认上传以上 $($builtPackages.Count) 个签名插件/皮肤包，并更新 Marketplace？"
}
if (-not (Read-YesNo $confirmationPrompt)) {
    Write-Host "已取消；本地插件包保留在 $OutputRoot"
    return
}

foreach ($plan in $builtPackages) { Publish-GitHubRelease -Plan $plan -SourceCommit $sourceCommit }
Write-Host ""
Write-Host "所选官方插件/皮肤包已上传。" -ForegroundColor Green

if (-not $SkipMarketplaceUpdate) {
    Update-MarketplaceAfterPublish -PublishedPackages $builtPackages -Context $marketplaceContext
    Remove-OldPluginReleases -Releases $releaseCleanupPlan
}
else {
    Write-Host "已跳过 Marketplace 更新和旧版清理；如需保持每个插件仅一个版本，请重新运行并移除 -SkipMarketplaceUpdate。" -ForegroundColor Yellow
}
