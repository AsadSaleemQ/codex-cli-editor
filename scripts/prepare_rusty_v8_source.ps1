[CmdletBinding()]
param([Parameter(Mandatory)] [string] $UpstreamRoot)

$ErrorActionPreference = 'Stop'
$commit = '90a65f887a0c0f048f3784afbcb320b373c8a2e0'
$digest = 'b0b188d1f1d851abd78337818a28a15e84f143b082ce966976e9206617684373'
$upstream = [IO.Path]::GetFullPath($UpstreamRoot)
$manifest = Join-Path $upstream 'codex-rs\Cargo.toml'
$original = 'v8 = { git = "https://github.com/openai-oss-forks/rusty_v8.git", rev = "' + $commit + '" }'
$replacement = 'v8 = { path = "../../rusty_v8-' + $commit + '" }'
$text = [IO.File]::ReadAllText($manifest)
$parent = Split-Path -Parent $upstream
$source = Join-Path $parent "rusty_v8-$commit"
if ($text.Contains($replacement) -and (Test-Path -LiteralPath (Join-Path $source 'Cargo.toml'))) { return }
if (!$text.Contains($original)) { throw 'Unexpected upstream V8 dependency declaration' }
$bazel = [IO.File]::ReadAllText((Join-Path $upstream 'MODULE.bazel'))
if (!$bazel.Contains($digest) -or !$bazel.Contains("rusty_v8/archive/$commit.tar.gz")) {
    throw 'Upstream V8 source archive provenance mismatch'
}

# Release builds link the prebuilt archive. Use the same Rust source as upstream Bazel
# without checking out Chromium submodules that exceed the Windows path limit.
$archive = Join-Path $parent "rusty_v8-$commit.tar.gz"
if (!(Test-Path -LiteralPath $archive)) {
    & curl.exe --fail --location --silent --show-error --retry 3 --output $archive "https://codeload.github.com/openai-oss-forks/rusty_v8/tar.gz/$commit"
    if ($LASTEXITCODE -ne 0) { throw 'V8 source download failed' }
}
if ((Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash -ne $digest) { throw 'V8 source archive checksum mismatch' }
if (Test-Path -LiteralPath $source) { throw "Refusing to overwrite V8 source directory: $source" }
& tar.exe -xzf $archive -C $parent
if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath (Join-Path $source 'Cargo.toml'))) { throw 'V8 source extraction failed' }
[IO.File]::WriteAllText($manifest, $text.Replace($original, $replacement), [Text.UTF8Encoding]::new($false))
