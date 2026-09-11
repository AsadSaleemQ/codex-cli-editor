[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $Version,
    [Parameter(Mandatory)] [uint64] $ManifestSequence,
    [Parameter(Mandatory)] [uint64] $ExpiresUnix,
    [uint64] $IssuedUnix = 0,
    [string] $CodexVersion = '0.154.0',
    [string] $Repository = 'AsadSaleemQ/codex-cli-editor',
    [string[]] $VsCodeVersions = @('1.134.0', '1.135.0', '1.137.0')
)

$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$upstreamCommit = '6b9826e3aa83b1a5947db50f4332cb9c65f1b340'
$patch = Join-Path $root 'patches\codex\rust-v0.154.0\0001-desktop-composer.patch'
$expectedPatchSha256 = 'a93ac22c790f7c3829f66906461cbaa10d0fe0e037a7bce727c7802260ed8b8e'
$work = Join-Path $root ".work\release-$Version"
$upstream = Join-Path $work 'codex'
$artifacts = Join-Path $root '.artifacts'
$bundle = Join-Path $artifacts "codex-cli-editor-$Version-windows-x64"
$archive = "$bundle.zip"
$releaseTag = "codex-cli-editor-v$Version-codex$CodexVersion"

function Get-Sha256([string] $Path) {
    $stream = [IO.File]::OpenRead($Path)
    try {
        $algorithm = [Security.Cryptography.SHA256]::Create()
        try { return ([BitConverter]::ToString($algorithm.ComputeHash($stream))).Replace('-', '').ToLowerInvariant() }
        finally { $algorithm.Dispose() }
    }
    finally { $stream.Dispose() }
}

if ($Repository -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$') { throw 'Repository must be an owner/name slug' }
if ((Get-Sha256 $patch) -ne $expectedPatchSha256) { throw 'Codex patch SHA-256 does not match the pinned provenance record' }
if ($ManifestSequence -eq 0) { throw 'ManifestSequence must be positive' }
$now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
if ($IssuedUnix -eq 0) { $IssuedUnix = [uint64]$now }
if ($IssuedUnix -gt $now + 300) { throw 'IssuedUnix cannot be more than five minutes in the future' }
if ($ExpiresUnix -le $IssuedUnix) { throw 'ExpiresUnix must be in the future' }
foreach ($target in @($work, $bundle, $archive)) {
    if (Test-Path -LiteralPath $target) { throw "Refusing to overwrite existing release path: $target" }
}
[IO.Directory]::CreateDirectory($work) | Out-Null
[IO.Directory]::CreateDirectory($artifacts) | Out-Null

& git clone --filter=blob:none --no-checkout https://github.com/openai/codex.git $upstream
if ($LASTEXITCODE -ne 0) { throw 'upstream clone failed' }
& git -C $upstream checkout $upstreamCommit
if ($LASTEXITCODE -ne 0) { throw 'upstream checkout failed' }
$upstreamConfig = Join-Path $upstream 'codex-rs\.cargo\config.toml'
$expectedUpstreamMsvcRustflags = 'rustflags = ["-C", "link-arg=/STACK:8388608", "-C", "target-feature=+crt-static"]'
$configLines = @(Get-Content -LiteralPath $upstreamConfig)
$msvcSection = [Array]::IndexOf($configLines, '[target.''cfg(all(windows, target_env = "msvc"))'']')
if ($msvcSection -lt 0 -or $msvcSection + 1 -ge $configLines.Count -or $configLines[$msvcSection + 1] -ne $expectedUpstreamMsvcRustflags) {
    throw 'pinned upstream MSVC rustflags changed; update the release builder before shipping'
}
& git -C $upstream apply --check $patch
if ($LASTEXITCODE -ne 0) { throw 'patch preflight failed' }
& git -C $upstream apply $patch
if ($LASTEXITCODE -ne 0) { throw 'patch application failed' }
& git -C $upstream diff --check
if ($LASTEXITCODE -ne 0) { throw 'patched tree has whitespace errors' }
& (Join-Path $root 'scripts\refresh_codex_lock.ps1') -UpstreamRoot $upstream
& (Join-Path $root 'scripts\prepare_i18n_embed_fl.ps1') -UpstreamRoot $upstream -WorkRoot $work
$rustyV8 = & (Join-Path $root 'scripts\prepare_rusty_v8.ps1') `
    -UpstreamRoot $upstream `
    -DestinationRoot (Join-Path $work 'rusty-v8')

$env:SOURCE_DATE_EPOCH = [string]$IssuedUnix
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_PROFILE_RELEASE_INCREMENTAL = 'false'
$env:CARGO_PROFILE_RELEASE_DEBUG = '0'
$env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = '1'
$deterministicRustflags = @(
    "--remap-path-prefix=$root=Z:/codex-cli-editor",
    "--remap-path-prefix=$upstream=Z:/codex",
    '-C',
    'link-arg=/Brepro'
)
$env:CARGO_ENCODED_RUSTFLAGS = (@(
    '-C',
    'target-feature=+crt-static'
) + $deterministicRustflags) -join [char]0x1f
& cargo +1.95.0-x86_64-pc-windows-msvc build --locked --release --bins --manifest-path (Join-Path $root 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw 'dispatcher build failed' }
$env:RUSTY_V8_ARCHIVE = $rustyV8.ArchivePath
$env:RUSTY_V8_SRC_BINDING_PATH = $rustyV8.BindingPath
$env:CARGO_ENCODED_RUSTFLAGS = (@(
    '-C',
    'link-arg=/STACK:8388608',
    '-C',
    'target-feature=+crt-static'
) + $deterministicRustflags) -join [char]0x1f
& cargo +1.95.0-x86_64-pc-windows-msvc build --locked --release --manifest-path (Join-Path $upstream 'codex-rs\Cargo.toml') -p codex-cli -p codex-code-mode-host
if ($LASTEXITCODE -ne 0) { throw 'patched Codex build failed' }

[IO.Directory]::CreateDirectory($bundle) | Out-Null
$vscodeExtensionName = 'codex-cli-editor.vsix'
& (Join-Path $root 'scripts\build_vscode_extension.ps1') -OutputPath (Join-Path $bundle $vscodeExtensionName)
$files = [ordered]@{
    'codex-cli-editor.exe' = Join-Path $root 'target\release\codex-cli-editor.exe'
    'codex-enhanced.exe' = Join-Path $upstream 'codex-rs\target\release\codex.exe'
    'codex-code-mode-host.exe' = Join-Path $upstream 'codex-rs\target\release\codex-code-mode-host.exe'
}
$releaseTools = Join-Path $artifacts 'release-tools'
[IO.Directory]::CreateDirectory($releaseTools) | Out-Null
$signer = Join-Path $root 'target\release\sign_release.exe'
if (-not (Test-Path -LiteralPath $signer -PathType Leaf)) { throw "Missing release signer: $signer" }
[IO.File]::Copy($signer, (Join-Path $releaseTools 'sign_release.exe'), $false)
foreach ($entry in $files.GetEnumerator()) {
    if (-not (Test-Path -LiteralPath $entry.Value -PathType Leaf)) { throw "Missing release artifact: $($entry.Value)" }
    [IO.File]::Copy($entry.Value, (Join-Path $bundle $entry.Key), $false)
}
& cargo +1.95.0-x86_64-pc-windows-msvc fetch --locked --manifest-path (Join-Path $root 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw 'dispatcher dependency fetch failed before frozen license generation' }
& cargo +1.95.0-x86_64-pc-windows-msvc fetch --locked --manifest-path (Join-Path $upstream 'codex-rs\Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw 'upstream dependency fetch failed before frozen license generation' }
$licenseReports = @(
    @{ Manifest = (Join-Path $root 'Cargo.toml'); Output = 'THIRD_PARTY_LICENSES_CODEX_CLI_EDITOR.html' },
    @{ Manifest = (Join-Path $upstream 'codex-rs\Cargo.toml'); Output = 'THIRD_PARTY_LICENSES_CODEX.html' }
)
foreach ($report in $licenseReports) {
    $output = Join-Path $bundle $report.Output
    & cargo about generate --frozen --config (Join-Path $root 'about.toml') --manifest-path $report.Manifest -o $output (Join-Path $root 'about.hbs')
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $output -PathType Leaf)) {
        throw "third-party license generation failed for $($report.Manifest)"
    }
}
$distributionDocs = @('LICENSE', 'NOTICE', 'THIRD_PARTY_NOTICES.md')
foreach ($name in $distributionDocs) {
    $source = Join-Path $root $name
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Missing distribution notice: $source" }
    [IO.File]::Copy($source, (Join-Path $bundle $name), $false)
}
$sysroot = (& rustc +1.95.0-x86_64-pc-windows-msvc --print sysroot).Trim()
$hostLine = & rustc +1.95.0-x86_64-pc-windows-msvc -vV | Where-Object { $_ -like 'host:*' }
$hostTriple = ($hostLine -split ':', 2)[1].Trim()
$llvmTools = Join-Path $sysroot "lib\rustlib\$hostTriple\bin"
$strip = @('llvm-strip.exe', 'llvm-objcopy.exe') |
    ForEach-Object { Join-Path $llvmTools $_ } |
    Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
    Select-Object -First 1
if (-not $strip) { throw "llvm-tools-preview strip utility not found under $llvmTools" }
$readObject = Join-Path $llvmTools 'llvm-readobj.exe'
if (-not (Test-Path -LiteralPath $readObject -PathType Leaf)) { throw "llvm-readobj.exe not found under $llvmTools" }
foreach ($name in $files.Keys) {
    & $strip --strip-all (Join-Path $bundle $name)
    if ($LASTEXITCODE -ne 0) { throw "symbol stripping failed for $name" }
}
$dispatcherVersion = & (Join-Path $bundle 'codex-cli-editor.exe') --version
if ($LASTEXITCODE -ne 0 -or ($dispatcherVersion -join ' ') -notmatch [regex]::Escape($Version)) {
    throw "Codex CLI Editor smoke test did not report $Version"
}
$reported = & (Join-Path $bundle 'codex-enhanced.exe') --version
if ($LASTEXITCODE -ne 0 -or ($reported -join ' ') -notmatch [regex]::Escape($CodexVersion)) {
    throw "Enhanced Codex smoke test did not report $CodexVersion"
}
$null = & (Join-Path $bundle 'codex-code-mode-host.exe') --help
if ($LASTEXITCODE -ne 0) { throw 'Code-mode host smoke test failed' }
foreach ($name in $files.Keys) {
    $artifact = Join-Path $bundle $name
    $imports = @(& $readObject --coff-imports $artifact 2>&1)
    if ($LASTEXITCODE -ne 0) { throw "PE import inspection failed for $name`: $($imports -join ' ')" }
    if (($imports -join "`n") -match '(?im)^\s*Name:\s*(?:VCRUNTIME|MSVCP)[^\s]*\.dll\s*$') {
        throw "$name imports the dynamic Microsoft C/C++ runtime"
    }
}
foreach ($name in @('codex-enhanced.exe', 'codex-code-mode-host.exe')) {
    $headers = @(& $readObject --file-headers (Join-Path $bundle $name) 2>&1)
    if ($LASTEXITCODE -ne 0) { throw "PE header inspection failed for $name`: $($headers -join ' ')" }
    if (($headers -join "`n") -notmatch '(?m)^\s*SizeOfStackReserve:\s*8388608\s*$') {
        throw "$name does not reserve the pinned 8 MiB main-thread stack"
    }
}

$artifactRecords = foreach ($name in @($files.Keys) + $vscodeExtensionName) {
    $file = Get-Item -LiteralPath (Join-Path $bundle $name)
    [ordered]@{
        name = $name
        url = "https://github.com/$Repository/releases/download/$releaseTag/$name"
        sha256 = Get-Sha256 $file.FullName
        size = [uint64]$file.Length
    }
}
$manifest = [ordered]@{
    schema_version = 1
    sequence = $ManifestSequence
    issued_unix = $IssuedUnix
    expires_unix = $ExpiresUnix
    minimum_dispatcher_version = $Version
    compatibility = @([ordered]@{
        codex = $CodexVersion
        vscode = $VsCodeVersions
    })
    artifacts = @($artifactRecords)
}
$manifestPath = Join-Path $bundle 'compatibility-manifest.json'
[IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 8) + "`n", (New-Object Text.UTF8Encoding($false)))

$dispatcherMetadata = (& cargo +1.95.0-x86_64-pc-windows-msvc metadata --locked --format-version 1 --manifest-path (Join-Path $root 'Cargo.toml') | ConvertFrom-Json)
$upstreamMetadata = (& cargo +1.95.0-x86_64-pc-windows-msvc metadata --locked --format-version 1 --manifest-path (Join-Path $upstream 'codex-rs\Cargo.toml') | ConvertFrom-Json)
$components = @($dispatcherMetadata.packages + $upstreamMetadata.packages) |
    Sort-Object name, version, source -Unique |
    ForEach-Object {
        $component = [ordered]@{
            type = 'library'
            'bom-ref' = "pkg:cargo/$($_.name)@$($_.version)"
            name = $_.name
            version = $_.version
            purl = "pkg:cargo/$($_.name)@$($_.version)"
        }
        if ($_.license) { $component.licenses = @([ordered]@{ expression = $_.license }) }
        if ($_.source) { $component.properties = @([ordered]@{ name = 'cargo:source'; value = $_.source }) }
        $component
    }
$manifestDigest = Get-Sha256 $manifestPath
$guidBytes = New-Object byte[] 16
[Array]::Copy(([Convert]::FromHexString($manifestDigest)), $guidBytes, 16)
$sbom = [ordered]@{
    bomFormat = 'CycloneDX'
    specVersion = '1.5'
    serialNumber = "urn:uuid:$([Guid]::new($guidBytes))"
    version = 1
    metadata = [ordered]@{
        timestamp = [DateTimeOffset]::FromUnixTimeSeconds([int64]$IssuedUnix).UtcDateTime.ToString('yyyy-MM-ddTHH:mm:ssZ')
        component = [ordered]@{
            type = 'application'
            name = 'codex-cli-editor'
            version = $Version
            properties = @(
                [ordered]@{ name = 'codex-cli-editor:codex-upstream-commit'; value = $upstreamCommit },
                [ordered]@{ name = 'codex-cli-editor:manifest-sha256'; value = $manifestDigest }
            )
        }
    }
    components = @($components)
}
$sbomPath = Join-Path $artifacts "codex-cli-editor-$Version.sbom.json"
[IO.File]::WriteAllText($sbomPath, ($sbom | ConvertTo-Json -Depth 12) + "`n", (New-Object Text.UTF8Encoding($false)))
$sourceArchive = Join-Path $artifacts "codex-cli-editor-$Version-source.zip"
& git -C $root archive --format=zip --output=$sourceArchive HEAD
if ($LASTEXITCODE -ne 0) { throw 'source archive generation failed' }
$expectedBundleFiles = @($files.Keys) + $vscodeExtensionName + $distributionDocs + @($licenseReports.Output) + @('compatibility-manifest.json')
$actualBundleFiles = @(Get-ChildItem -LiteralPath $bundle -File | ForEach-Object Name | Sort-Object)
$missingBundleFiles = @($expectedBundleFiles | Where-Object { $_ -notin $actualBundleFiles })
if ($missingBundleFiles.Count -ne 0) { throw "Unsigned release bundle is incomplete: $($missingBundleFiles -join ', ')" }
$unexpectedBundleFiles = @($actualBundleFiles | Where-Object { $_ -notin $expectedBundleFiles })
if ($unexpectedBundleFiles.Count -ne 0) { throw "Unsigned release bundle has unexpected files: $($unexpectedBundleFiles -join ', ')" }
$expectedArtifactEntries = @([IO.Path]::GetFileName($bundle), 'release-tools', [IO.Path]::GetFileName($sbomPath), [IO.Path]::GetFileName($sourceArchive))
$actualArtifactEntries = @(Get-ChildItem -LiteralPath $artifacts | ForEach-Object Name)
$artifactInventoryDelta = @($expectedArtifactEntries | Where-Object { $_ -notin $actualArtifactEntries }) + @($actualArtifactEntries | Where-Object { $_ -notin $expectedArtifactEntries })
if ($artifactInventoryDelta.Count -ne 0) { throw "Unsigned artifact inventory mismatch: $($artifactInventoryDelta -join ', ')" }
Write-Output "Unsigned release assets: $artifacts"
Write-Output "Manifest ready for isolated signing: $manifestPath"
