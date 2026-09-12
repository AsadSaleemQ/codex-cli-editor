[CmdletBinding()]
param(
    [Parameter(Mandatory)] [uint64] $RunId,
    [Parameter(Mandatory)] [string] $SigningKeyPath,
    [string] $Repository = 'AsadSaleemQ/codex-cli-editor',
    [double] $MinimumFreeGiB = 5
)

$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$artifacts = Join-Path $root '.artifacts'
$ownedArtifacts = $false

function Invoke-Checked([string] $Program, [string[]] $Arguments, [string] $Failure) {
    $output = @(& $Program @Arguments 2>&1)
    if ($LASTEXITCODE -ne 0) { throw "$Failure`: $($output -join ' ')" }
    $output
}

function Get-FreeGiB([string] $Path) {
    $driveRoot = [IO.Path]::GetPathRoot([IO.Path]::GetFullPath($Path))
    ([IO.DriveInfo]::new($driveRoot)).AvailableFreeSpace / 1GB
}

if ($Repository -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$') { throw 'Repository must be an owner/name slug' }
if ($MinimumFreeGiB -lt 1) { throw 'MinimumFreeGiB cannot be less than one' }
$keyPath = [IO.Path]::GetFullPath($SigningKeyPath)
if (-not (Test-Path -LiteralPath $keyPath -PathType Leaf)) { throw "Signing key does not exist: $keyPath" }
$keyHex = ([IO.File]::ReadAllText($keyPath)).Trim()
if ($keyHex -notmatch '^[0-9a-fA-F]{64}$') { throw 'Signing key must contain exactly 32 bytes encoded as hexadecimal' }
if (Test-Path -LiteralPath $artifacts) { throw "Refusing to overwrite the local artifact workspace: $artifacts" }
$freeBefore = Get-FreeGiB $root
if ($freeBefore -lt $MinimumFreeGiB) { throw ("Release download requires at least {0:N1} GiB free; only {1:N1} GiB is available" -f $MinimumFreeGiB, $freeBefore) }

try {
    [IO.Directory]::CreateDirectory($artifacts) | Out-Null
    $ownedArtifacts = $true
    $run = (Invoke-Checked 'gh' @('run', 'view', [string]$RunId, '--repo', $Repository, '--json', 'status,conclusion,headSha,workflowName,url') 'Unable to inspect release run') -join "`n" | ConvertFrom-Json
    if ($run.status -ne 'completed' -or $run.conclusion -ne 'success' -or $run.workflowName -ne 'release') {
        throw "Run $RunId is not a successful completed release workflow run"
    }
    $head = ((Invoke-Checked 'git' @('-C', $root, 'rev-parse', 'HEAD') 'Unable to read local Git HEAD') -join '').Trim()
    if ($head -ne [string]$run.headSha) { throw "Run commit $($run.headSha) does not match local HEAD $head" }
    Invoke-Checked 'gh' @('run', 'download', [string]$RunId, '--repo', $Repository, '--name', 'primary-unsigned-assets', '--dir', $artifacts) 'Unable to download the unsigned release candidate' | Out-Null

    $manifests = @(Get-ChildItem -LiteralPath $artifacts -Recurse -File -Filter 'compatibility-manifest.json')
    if ($manifests.Count -ne 1) { throw 'Unsigned candidate must contain exactly one compatibility manifest' }
    $manifest = Get-Content -LiteralPath $manifests[0].FullName -Raw | ConvertFrom-Json
    $version = [string]$manifest.minimum_dispatcher_version
    $sequence = [uint64]$manifest.sequence
    $issued = [uint64]$manifest.issued_unix
    $expires = [uint64]$manifest.expires_unix
    $codexVersions = @($manifest.compatibility | ForEach-Object { [string]$_.codex } | Sort-Object -Unique)
    if ($version -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$' -or $codexVersions.Count -ne 1) {
        throw 'Unsigned candidate has invalid release version metadata'
    }
    $bundle = Split-Path -Parent $manifests[0].FullName
    if ((Split-Path -Leaf $bundle) -ne "codex-cli-editor-$version-windows-x64") { throw 'Unsigned bundle directory name does not match its manifest version' }

    $env:CODEX_CLI_EDITOR_SIGNING_PRIVATE_KEY_HEX = $keyHex
    try {
        & (Join-Path $root 'scripts\finalize_release.ps1') -Version $version -IssuedUnix $issued -ManifestSequence $sequence -ExpiresUnix $expires -RepositoryRoot $root
        if ($LASTEXITCODE -ne 0) { throw 'Local release finalization failed' }
    } finally {
        Remove-Item Env:\CODEX_CLI_EDITOR_SIGNING_PRIVATE_KEY_HEX -ErrorAction SilentlyContinue
        $keyHex = $null
    }

    $tag = "codex-cli-editor-v$version-codex$($codexVersions[0])"
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        # Windows PowerShell 5.1 promotes native stderr to ErrorRecord objects when
        # ErrorActionPreference is Stop. A missing release is the expected result
        # of this guard, so collect the command result under Continue and validate
        # its exit code and message explicitly.
        $ErrorActionPreference = 'Continue'
        $existing = @(& gh release view $tag --repo $Repository 2>&1)
        $existingExitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    if ($existingExitCode -eq 0) { throw "Release tag already exists: $tag" }
    $existingMessage = ($existing -join ' ')
    if ($existingExitCode -ne 1 -or $existingMessage -notmatch '(?i)release not found') {
        throw "Unable to check whether release tag exists: $existingMessage"
    }
    $assetNames = @(
        "codex-cli-editor-$version-windows-x64.zip",
        "codex-cli-editor-$version-windows-x64.zip.sha256",
        "codex-cli-editor-$version.sbom.json",
        "codex-cli-editor-$version-source.zip",
        'codex-cli-editor.exe',
        'codex-enhanced.exe',
        'codex-code-mode-host.exe',
        'codex-cli-editor.vsix',
        'compatibility-manifest.json',
        'compatibility-manifest.sig'
    )
    $assets = foreach ($name in $assetNames) {
        $matches = @(Get-ChildItem -LiteralPath $artifacts -Recurse -File -Filter $name)
        if ($matches.Count -ne 1) { throw "Final release must contain exactly one $name" }
        $matches[0].FullName
    }
    $releaseNotes = Join-Path $root 'RELEASE_NOTES.md'
    Invoke-Checked 'gh' (@('release', 'create', $tag) + $assets + @('--repo', $Repository, '--target', $head, '--title', "Codex CLI Editor v$version", '--notes-file', $releaseNotes, '--draft')) 'Unable to publish the signed draft release' | Out-Null
    Write-Output "Published signed draft release $tag from $($run.url)"
} finally {
    Remove-Item Env:\CODEX_CLI_EDITOR_SIGNING_PRIVATE_KEY_HEX -ErrorAction SilentlyContinue
    $keyHex = $null
    if ($ownedArtifacts -and (Test-Path -LiteralPath $artifacts)) {
        Remove-Item -LiteralPath $artifacts -Recurse -Force
    }
    $freeAfter = Get-FreeGiB $root
    Write-Output ("Local release workspace removed; free space is {0:N2} GiB" -f $freeAfter)
}
