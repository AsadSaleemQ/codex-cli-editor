[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $UpstreamRoot,
    [string] $Toolchain = '1.95.0-x86_64-pc-windows-msvc',
    [string] $TargetTriple = 'x86_64-pc-windows-msvc',
    [switch] $Offline
)

$ErrorActionPreference = 'Stop'
$manifest = Join-Path $UpstreamRoot 'codex-rs\Cargo.toml'
if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) { throw "Missing upstream manifest: $manifest" }

$arguments = @("+$Toolchain", 'metadata', '--manifest-path', $manifest, '--format-version', '1', '--filter-platform', $TargetTriple)
if ($Offline) { $arguments += '--offline' }
$null = & cargo @arguments
if ($LASTEXITCODE -ne 0) { throw 'Unable to refresh the pinned upstream lockfile.' }
$diff = @(& git -C $UpstreamRoot diff --unified=0 --no-color -- codex-rs/Cargo.lock)
if ($LASTEXITCODE -ne 0) { throw 'Unable to inspect the refreshed upstream lockfile.' }
$removed = @($diff | Where-Object { $_ -eq '-version = "0.0.0"' })
$added = @($diff | Where-Object { $_ -eq '+version = "0.154.0"' })
$unexpected = @($diff | Where-Object {
    ($_ -like '+*' -or $_ -like '-*') -and
    $_ -notlike '+++*' -and
    $_ -notlike '---*' -and
    $_ -ne '-version = "0.0.0"' -and
    $_ -ne '+version = "0.154.0"'
})
if ($removed.Count -eq 0 -or $removed.Count -ne $added.Count -or $unexpected.Count -ne 0) {
    if ($unexpected.Count -ne 0) { Write-Error "Unexpected lockfile drift: $($unexpected -join '; ')" }
    throw "Pinned Codex lock refresh changed more than local package versions ($($removed.Count) removed, $($added.Count) added)."
}
Write-Output "Validated pinned Codex lock refresh for $($added.Count) local workspace packages; dependency versions are unchanged."
