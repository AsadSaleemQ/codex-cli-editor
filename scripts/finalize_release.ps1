[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $Version,
    [Parameter(Mandatory)] [uint64] $IssuedUnix,
    [Parameter(Mandatory)] [uint64] $ManifestSequence,
    [Parameter(Mandatory)] [uint64] $ExpiresUnix,
    [string] $RepositoryRoot
)

$ErrorActionPreference = 'Stop'
$root = if ($RepositoryRoot) { [IO.Path]::GetFullPath($RepositoryRoot) } else { [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..')) }
$artifacts = Join-Path $root '.artifacts'
$bundle = Join-Path $artifacts "codex-cli-editor-$Version-windows-x64"
$archive = "$bundle.zip"
$manifestPath = Join-Path $bundle 'compatibility-manifest.json'
$signaturePath = Join-Path $bundle 'compatibility-manifest.sig'
$generatedPublicKey = Join-Path $artifacts 'public-key.generated.hex'
$signer = Join-Path $artifacts 'release-tools\sign_release.exe'

function Get-Sha256([string] $Path) {
    $stream = [IO.File]::OpenRead($Path)
    try {
        $algorithm = [Security.Cryptography.SHA256]::Create()
        try { return ([BitConverter]::ToString($algorithm.ComputeHash($stream))).Replace('-', '').ToLowerInvariant() }
        finally { $algorithm.Dispose() }
    }
    finally { $stream.Dispose() }
}

function New-DeterministicZip([string] $SourceDirectory, [string] $Destination, [uint64] $TimestampUnix) {
    Add-Type -AssemblyName System.IO.Compression
    $timestamp = [DateTimeOffset]::FromUnixTimeSeconds([int64]$TimestampUnix)
    if ($timestamp.Year -lt 1980) { $timestamp = [DateTimeOffset]'1980-01-01T00:00:00Z' }
    $stream = [IO.File]::Open($Destination, [IO.FileMode]::CreateNew, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try {
        $zip = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Create, $true)
        try {
            $sourceRoot = [IO.Path]::GetFullPath($SourceDirectory).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
            foreach ($file in Get-ChildItem -LiteralPath $SourceDirectory -File -Recurse | Sort-Object FullName) {
                $relative = $file.FullName.Substring($sourceRoot.Length).Replace('\', '/')
                $entry = $zip.CreateEntry($relative, [IO.Compression.CompressionLevel]::Optimal)
                $entry.LastWriteTime = $timestamp
                $input = [IO.File]::OpenRead($file.FullName)
                $output = $entry.Open()
                try { $input.CopyTo($output) }
                finally { $output.Dispose(); $input.Dispose() }
            }
        }
        finally { $zip.Dispose() }
    }
    finally { $stream.Dispose() }
}

if (-not $env:CODEX_CLI_EDITOR_SIGNING_PRIVATE_KEY_HEX) { throw 'CODEX_CLI_EDITOR_SIGNING_PRIVATE_KEY_HEX is required' }
foreach ($required in @($bundle, $manifestPath, $signer)) {
    if (-not (Test-Path -LiteralPath $required)) { throw "Unsigned release input is missing: $required" }
}
foreach ($target in @($signaturePath, $generatedPublicKey, $archive, "$archive.sha256")) {
    if (Test-Path -LiteralPath $target) { throw "Refusing to overwrite final release path: $target" }
}
$manifestBytes = [IO.File]::ReadAllBytes($manifestPath)
$manifestText = [Text.Encoding]::UTF8.GetString($manifestBytes)
$schemaPath = Join-Path $root 'compatibility\manifest.schema.json'
$testJson = Get-Command Test-Json -ErrorAction SilentlyContinue
if ($testJson -and -not ($manifestText | Test-Json -SchemaFile $schemaPath -ErrorAction Stop)) { throw 'unsigned manifest failed schema validation' }
try { $manifest = $manifestText | ConvertFrom-Json }
catch { throw 'unsigned manifest is not valid JSON' }
$requiredManifestProperties = @('schema_version', 'sequence', 'issued_unix', 'expires_unix', 'minimum_dispatcher_version', 'compatibility', 'artifacts')
$actualManifestProperties = @($manifest.PSObject.Properties.Name)
if (@($requiredManifestProperties | Where-Object { $_ -notin $actualManifestProperties }).Count -ne 0 -or
    @($actualManifestProperties | Where-Object { $_ -notin $requiredManifestProperties }).Count -ne 0 -or
    [uint64]$manifest.schema_version -ne 1 -or [uint64]$manifest.sequence -lt 1 -or
    -not ($manifest.compatibility -is [Array]) -or -not ($manifest.artifacts -is [Array])) {
    throw 'unsigned manifest failed schema validation'
}
if ([string]$manifest.minimum_dispatcher_version -ne $Version) { throw 'manifest dispatcher version does not match release version' }
if ([uint64]$manifest.issued_unix -ne $IssuedUnix) { throw 'manifest timestamp does not match the prepared release timestamp' }
if ([uint64]$manifest.sequence -ne $ManifestSequence) { throw 'manifest sequence does not match the prepared release sequence' }
if ([uint64]$manifest.expires_unix -ne $ExpiresUnix) { throw 'manifest expiry does not match the prepared release expiry' }
if ($ExpiresUnix -le $IssuedUnix) { throw 'manifest expiry must follow its issue timestamp' }
$artifactPaths = @{
    'codex-cli-editor.exe' = Join-Path $bundle 'codex-cli-editor.exe'
    'codex-enhanced.exe' = Join-Path $bundle 'codex-enhanced.exe'
    'codex-code-mode-host.exe' = Join-Path $bundle 'codex-code-mode-host.exe'
    'codex-cli-editor.vsix' = Join-Path $bundle 'codex-cli-editor.vsix'
}
$expectedArtifactNames = @($artifactPaths.Keys | Sort-Object)
$actualArtifactNames = @($manifest.artifacts | ForEach-Object { [string]$_.name } | Sort-Object)
if (($actualArtifactNames -join "`n") -ne ($expectedArtifactNames -join "`n")) {
    throw 'manifest artifact names must match the exact unique release inventory'
}
foreach ($artifact in $manifest.artifacts) {
    $path = $artifactPaths[[string]$artifact.name]
    if (-not $path -or -not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "manifest declares an unknown or missing artifact: $($artifact.name)" }
    $item = Get-Item -LiteralPath $path
    if ([uint64]$artifact.size -ne [uint64]$item.Length -or [string]$artifact.sha256 -ne (Get-Sha256 $path)) {
        throw "unsigned artifact verification failed: $($artifact.name)"
    }
}
& $signer - $manifestPath $signaturePath $generatedPublicKey
if ($LASTEXITCODE -ne 0) { throw 'isolated manifest signing failed' }
$expectedKey = ([IO.File]::ReadAllText((Join-Path $root 'compatibility\public-key.hex'))).Trim()
$actualKey = ([IO.File]::ReadAllText($generatedPublicKey)).Trim()
if ($actualKey -ne $expectedKey) { throw 'signing key does not match the embedded release public key' }
$requiredBundleFiles = @(
    'codex-cli-editor.exe', 'codex-enhanced.exe', 'codex-code-mode-host.exe', 'codex-cli-editor.vsix',
    'compatibility-manifest.json', 'compatibility-manifest.sig',
    'LICENSE', 'NOTICE', 'THIRD_PARTY_NOTICES.md',
    'THIRD_PARTY_LICENSES_CODEX_CLI_EDITOR.html', 'THIRD_PARTY_LICENSES_CODEX.html'
)
$actualBundleFiles = @(Get-ChildItem -LiteralPath $bundle -File | ForEach-Object Name)
$missing = @($requiredBundleFiles | Where-Object { $_ -notin $actualBundleFiles })
if ($missing.Count -ne 0) { throw "signed release bundle is incomplete: $($missing -join ', ')" }
$unexpected = @($actualBundleFiles | Where-Object { $_ -notin $requiredBundleFiles })
if ($unexpected.Count -ne 0) { throw "signed release bundle has unexpected files: $($unexpected -join ', ')" }
New-DeterministicZip -SourceDirectory $bundle -Destination $archive -TimestampUnix $IssuedUnix
$archiveHash = Get-Sha256 $archive
[IO.File]::WriteAllText("$archive.sha256", "$archiveHash  $([IO.Path]::GetFileName($archive))`n", (New-Object Text.UTF8Encoding($false)))
$expectedArtifactEntries = @([IO.Path]::GetFileName($bundle), 'release-tools', 'public-key.generated.hex', "codex-cli-editor-$Version.sbom.json", "codex-cli-editor-$Version-source.zip", [IO.Path]::GetFileName($archive), [IO.Path]::GetFileName("$archive.sha256"))
$actualArtifactEntries = @(Get-ChildItem -LiteralPath $artifacts | ForEach-Object Name)
$artifactInventoryDelta = @($expectedArtifactEntries | Where-Object { $_ -notin $actualArtifactEntries }) + @($actualArtifactEntries | Where-Object { $_ -notin $expectedArtifactEntries })
if ($artifactInventoryDelta.Count -ne 0) { throw "Signed artifact inventory mismatch: $($artifactInventoryDelta -join ', ')" }
Write-Output "Signed release bundle: $archive"
Write-Output "SHA-256: $archiveHash"
