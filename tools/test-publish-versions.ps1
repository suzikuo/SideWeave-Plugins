# Run with Windows PowerShell 5.1; all writes stay in an isolated temporary workspace.
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$wizardPath = Join-Path $repo 'scripts/publish-wizard.ps1'
$tokens = $null
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($wizardPath, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors -join "`n") }
foreach ($fn in $ast.FindAll({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    . ([scriptblock]::Create($fn.Extent.Text))
}
. (Join-Path $repo 'scripts/package-version.ps1')
$cargoExe = (Get-Command cargo -CommandType Application).Source
$RepoRoot = Join-Path ([IO.Path]::GetTempPath()) ('sideweave-version-test-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($RepoRoot)
$Repository = 'test/plugins'
$script:passed = 0
function Assert($Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Test([string]$Name, [scriptblock]$Run) {
    & $Run
    $script:passed++
    Write-Host "PASS $Name"
}
function Plugin([string]$Id, [string]$Version, [string]$Kind = 'plugin') {
    [PSCustomObject]@{ id = $Id; version = $Version; kind = $Kind }
}
function Reset-Tags {
    $script:remoteLines = @()
    $script:remoteExit = 0
    $script:localTags = ''
    $script:dirty = ''
}
function Get-GitText([string]$Path, [string[]]$Arguments) {
    if ($Arguments[0] -eq 'status') { return $script:dirty }
    if ($Arguments[0] -eq 'tag') { return $script:localTags }
    return 'aaaa'
}
function Invoke-GitCapture([string]$Path, [string[]]$Arguments) {
    [PSCustomObject]@{ ExitCode = $script:remoteExit; Output = $script:remoteLines }
}
function Write-Fixture([string]$Path, [string]$Text, [bool]$Bom = $false) {
    [void][IO.Directory]::CreateDirectory((Split-Path $Path -Parent))
    [IO.File]::WriteAllText($Path, $Text, [Text.UTF8Encoding]::new($Bom))
}
function Bytes([string]$Path) { [Convert]::ToBase64String([IO.File]::ReadAllBytes($Path)) }
Reset-Tags

try {
    Test 'unused version and identical source need no bump' {
        $p = Plugin quick_access 1.0.0
        Assert (@(Get-PackageVersionPlan @($p) $false 'aaaa').Count -eq 0) 'Unused version changed'
        $script:remoteLines = @('aaaa refs/tags/plugin-quick_access-v1.0.0')
        Assert (@(Get-PackageVersionPlan @($p) $false 'aaaa').Count -eq 0) 'Same commit changed'
    }
    Test 'conflicting source proposes an unused patch above all published versions' {
        $script:remoteLines = @('bbbb refs/tags/plugin-quick_access-v1.0.0', 'cccc refs/tags/plugin-quick_access-v1.0.3')
        $plan = @(Get-PackageVersionPlan @((Plugin quick_access 1.0.0)) $false 'aaaa')
        Assert ($plan.Count -eq 1 -and $plan[0].NewVersion -eq '1.0.4') 'Incorrect next patch'
    }
    Test 'annotated tags compare peeled commit regardless of row order' {
        $script:remoteLines = @('aaaa refs/tags/plugin-quick_access-v1.0.0^{}', 'bbbb refs/tags/plugin-quick_access-v1.0.0')
        Assert (@(Get-PackageVersionPlan @((Plugin quick_access 1.0.0)) $false 'aaaa').Count -eq 0) 'Compared annotated object instead of commit'
    }
    Test 'regression and prerelease conflicts advance safely' {
        $script:remoteLines = @('bbbb refs/tags/plugin-quick_access-v2.3.7-rc.1')
        $plan = @(Get-PackageVersionPlan @((Plugin quick_access 1.0.1)) $false 'aaaa')
        Assert ($plan[0].NewVersion -eq '2.3.8') 'Version would regress'
        Assert ((Get-NextPackageVersion '0.4.10-beta.1+build' @()) -eq '0.4.11') 'Invalid prerelease patch'
    }
    Test 'batch bump includes occupied same-source tags but leaves fresh versions alone' {
        $script:remoteLines = @('bbbb refs/tags/plugin-quick_access-v1.0.0', 'aaaa refs/tags/plugin-ai_usage-v0.4.10')
        $plan = @(Get-PackageVersionPlan @((Plugin quick_access 1.0.0), (Plugin ai_usage 0.4.10), (Plugin time 1.2.0)) $false 'aaaa')
        Assert ($plan.Count -eq 2) 'Batch version plan is incomplete'
        Assert ($plan[1].NewVersion -eq '0.4.11') 'Existing batch tag not advanced'
    }
    Test 'offline local builds use local tags and include uncommitted source' {
        Reset-Tags
        $script:remoteExit = 1
        $script:localTags = 'plugin-quick_access-v1.0.0'
        $script:dirty = ' M plugins/quick_access/src/lib.rs'
        $plan = @(Get-PackageVersionPlan @((Plugin quick_access 1.0.0)) $true 'aaaa' -WarningAction SilentlyContinue)
        Assert ($plan.Count -eq 1) 'Dirty local source reused a published version'
        $failed = $false
        try { Get-PackageVersionPlan @((Plugin quick_access 1.0.0)) $false 'aaaa' }
        catch { $failed = $true }
        Assert $failed 'Publishing ignored remote query failure'
    }

    # Real Cargo, no dependencies or network. Include both direct and inherited versions.
    Write-Fixture (Join-Path $RepoRoot 'Cargo.toml') "[workspace]`nmembers = [`"plugins/direct`", `"plugins/inherited`"]`nresolver = `"2`"`n[workspace.package]`nversion = `"1.0.0`"`n"
    foreach ($id in @('direct', 'inherited')) {
        $versionField = if ($id -eq 'direct') { 'version = "1.0.0" # keep comment' } else { 'version.workspace = true # keep comment' }
        Write-Fixture (Join-Path $RepoRoot "plugins/$id/Cargo.toml") "[package]`r`nname = `"test-$id`"`r`n$versionField`r`nedition = `"2021`"`r`n"
        Write-Fixture (Join-Path $RepoRoot "plugins/$id/src/lib.rs") ''
        Write-Fixture (Join-Path $RepoRoot "plugins/$id/manifest.json") "{`r`n  `"id`": `"$id`",`r`n  `"version`": `"1.0.0`"`r`n}`r`n" ($id -eq 'direct')
    }
    $null = & $cargoExe generate-lockfile --manifest-path (Join-Path $RepoRoot Cargo.toml) --offline --quiet
    Assert ($LASTEXITCODE -eq 0) 'Fixture Cargo lock generation failed'
    Test 'native bump updates manifests, direct/inherited Cargo versions, and Cargo.lock' {
        $direct = Plugin direct 1.0.0
        $inherited = Plugin inherited 1.0.0
        $plan = @(
            [PSCustomObject]@{ Plugin = $direct; OldVersion = '1.0.0'; NewVersion = '1.0.1' },
            [PSCustomObject]@{ Plugin = $inherited; OldVersion = '1.0.0'; NewVersion = '1.0.2' }
        )
        $paths = @(Update-PackageVersions $plan)
        Assert ($paths.Count -eq 5) 'Wrong changed files'
        $metadata = (& $cargoExe metadata --manifest-path (Join-Path $RepoRoot Cargo.toml) --locked --offline --format-version 1 --quiet) | ConvertFrom-Json
        Assert ($LASTEXITCODE -eq 0) 'Lockfile was not synchronized'
        Assert (($metadata.packages | Where-Object name -eq test-direct).version -eq '1.0.1') 'Direct Cargo version mismatch'
        Assert (($metadata.packages | Where-Object name -eq test-inherited).version -eq '1.0.2') 'Inherited version mismatch'
        Assert ($direct.version -eq '1.0.1' -and $inherited.version -eq '1.0.2') 'Choice versions stale'
        Assert ([IO.File]::ReadAllText((Join-Path $RepoRoot Cargo.toml)).Contains('version = "1.0.0"')) 'Workspace version changed'
        $manifestPath = Join-Path $RepoRoot plugins/direct/manifest.json
        $data = [IO.File]::ReadAllBytes($manifestPath)
        Assert ($data[0] -eq 239 -and $data[1] -eq 187 -and $data[2] -eq 191) 'UTF8 BOM lost'
        Assert ([IO.File]::ReadAllText($manifestPath).Contains("`r`n")) 'CRLF lost'
        Assert ([IO.File]::ReadAllText((Join-Path $RepoRoot plugins/direct/Cargo.toml)).Contains('# keep comment')) 'Comment lost'
    }
    Test 'Cargo failure restores every original byte including lockfile' {
        $paths = @('plugins/direct/manifest.json', 'plugins/direct/Cargo.toml', 'Cargo.lock')
        $originals = @{}
        foreach ($path in $paths) { $originals[$path] = Bytes (Join-Path $RepoRoot $path) }
        $failed = $false
        $p = Plugin direct 1.0.1
        try {
            & {
                function cargo { $global:LASTEXITCODE = 1 }
                Update-PackageVersions @([PSCustomObject]@{ Plugin = $p; OldVersion = '1.0.1'; NewVersion = '1.0.2' })
            }
        }
        catch { $failed = $true }
        Assert $failed 'Cargo error ignored'
        foreach ($path in $paths) { Assert ((Bytes (Join-Path $RepoRoot $path)) -eq $originals[$path]) "Rollback failed: $path" }
        Assert ($p.version -eq '1.0.1') 'Failed update changed choice'
    }
    Test 'skin bump requires no Cargo files' {
        Write-Fixture (Join-Path $RepoRoot skins/demo/manifest.json) '{"id":"demo","version":"1.2.3"}'
        $p = Plugin demo 1.2.3 skin
        $paths = @(Update-PackageVersions @([PSCustomObject]@{ Plugin = $p; OldVersion = '1.2.3'; NewVersion = '1.2.4' }))
        Assert ($paths.Count -eq 1 -and $p.version -eq '1.2.4') 'Skin bump failed'
    }

    # Exercise the actual wizard preflight, without key access, builds, or uploads.
    $wizard = [IO.File]::ReadAllText($wizardPath)
    $start = $wizard.IndexOf('$headCommit =')
    $end = $wizard.IndexOf('$builtPackages =', $start)
    $preflight = [scriptblock]::Create($wizard.Substring($start, $end - $start))
    $script:updateCalls = 0
    $script:promptCalls = 0
    function Update-PackageVersions([object[]]$Plan) {
        $script:updateCalls++
        foreach ($item in $Plan) { $item.Plugin.version = $item.NewVersion }
        return 'fixture/manifest.json'
    }
    function Read-YesNo([string]$Prompt) { $script:promptCalls++; return $script:accept }
    function Assert-LocalSourceMatchesGitHub { throw 'Unexpected source validation after version update' }
    function Reset-Preflight {
        Reset-Tags
        $script:remoteLines = @('bbbb refs/tags/plugin-quick_access-v1.0.0')
        $script:updateCalls = 0
        $script:promptCalls = 0
    }
    Test 'preview never updates files even with AutoBumpVersion' {
        Reset-Preflight
        $selectedPlugins = @((Plugin quick_access 1.0.0))
        $LocalOnly = $false; $PreviewOnly = $true; $SkipBuild = $false; $AutoBumpVersion = $true
        & $preflight
        Assert ($script:updateCalls -eq 0 -and $script:promptCalls -eq 0) 'Preview mutated or prompted'
    }
    Test 'declining bump stops before build and source validation' {
        Reset-Preflight
        $selectedPlugins = @((Plugin quick_access 1.0.0))
        $LocalOnly = $false; $PreviewOnly = $false; $SkipBuild = $false; $AutoBumpVersion = $false; $script:accept = $false
        & $preflight
        Assert ($script:updateCalls -eq 0 -and $script:promptCalls -eq 1) 'Cancel mutated files'
    }
    Test 'SkipBuild conflict requires rebuilding before any version mutation' {
        Reset-Preflight
        $selectedPlugins = @((Plugin quick_access 1.0.0))
        $LocalOnly = $false; $PreviewOnly = $false; $SkipBuild = $true; $AutoBumpVersion = $true
        $failed = $false
        try { & $preflight } catch { $failed = $true }
        Assert ($failed -and $script:updateCalls -eq 0) 'SkipBuild bumped a version without rebuilding'
    }
    Test 'accepted publish bump stops for commit and push; auto mode skips prompt' {
        foreach ($auto in @($false, $true)) {
            Reset-Preflight
            $selectedPlugins = @((Plugin quick_access 1.0.0))
            $LocalOnly = $false; $PreviewOnly = $false; $SkipBuild = $false; $AutoBumpVersion = $auto; $script:accept = $true
            & $preflight
            Assert ($script:updateCalls -eq 1 -and $selectedPlugins[0].version -eq '1.0.1') 'Publish bump not applied'
            Assert ($script:promptCalls -eq [int](-not $auto)) 'Auto mode prompted'
        }
    }
    Test 'local automatic bump continues with the new version' {
        Reset-Preflight
        $selectedPlugins = @((Plugin quick_access 1.0.0))
        $LocalOnly = $true; $PreviewOnly = $false; $SkipBuild = $false; $AutoBumpVersion = $true
        $MinisignExe = Join-Path $RepoRoot Cargo.toml
        $SigningKeyPath = $MinisignExe # Existence check only; never reads a real key.
        & $preflight
        Assert ($script:updateCalls -eq 1 -and $selectedPlugins[0].version -eq '1.0.1') 'Local bump did not continue'
    }
    Write-Host "$script:passed tests passed. No builds, signing, or releases performed."
}
finally {
    # Only remove the exact test directory created above, after checking its boundary.
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    $testRoot = [IO.Path]::GetFullPath($RepoRoot)
    if (-not $testRoot.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -or (Split-Path $testRoot -Leaf) -notlike 'sideweave-version-test-*') {
        throw "Refusing cleanup outside temporary test directory: $testRoot"
    }
    Remove-Item -LiteralPath $testRoot -Recurse -Force
}
