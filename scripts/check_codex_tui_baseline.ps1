[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $UpstreamRoot,
    [string] $Toolchain = '1.95.0-x86_64-pc-windows-msvc',
    [int] $ExpectedPassed = 4245
)

$ErrorActionPreference = 'Stop'
$env:RUST_MIN_STACK = '16777216'
$env:INSTA_WORKSPACE_ROOT = [IO.Path]::GetFullPath((Join-Path $UpstreamRoot 'codex-rs'))
$env:INSTA_UPDATE = 'no'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$manifest = Join-Path $UpstreamRoot 'codex-rs\Cargo.toml'
$expectedPath = Join-Path $root 'compatibility\codex-tui-windows-baseline-failures.txt'
if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) { throw "Missing upstream manifest: $manifest" }
if (-not (Test-Path -LiteralPath $expectedPath -PathType Leaf)) { throw "Missing failure baseline: $expectedPath" }

$outputLines = [Collections.Generic.List[string]]::new()
$ErrorActionPreference = 'Continue'
& cargo "+$Toolchain" test --locked --manifest-path $manifest -p codex-tui --no-fail-fast 2>&1 |
    ForEach-Object {
        $line = $_.ToString()
        $outputLines.Add($line)
        Write-Output $line
    }
$testExit = $LASTEXITCODE
$ErrorActionPreference = 'Stop'
$text = $outputLines -join "`n"
$failureBlocks = [regex]::Matches($text, '(?ms)(?:^|\n)failures:\n(?<body>.*?)(?=\ntest result:)')
if ($failureBlocks.Count -eq 0) {
    if ($testExit -eq 0) {
        throw 'The patched suite unexpectedly has no failures; refresh the unpatched baseline before changing this gate.'
    }
    $tail = @($outputLines | Select-Object -Last 20) -join [Environment]::NewLine
    throw "Unable to locate any cargo test failure list. Cargo tail:$([Environment]::NewLine)$tail"
}
$actual = $failureBlocks |
    ForEach-Object { [regex]::Matches($_.Groups['body'].Value, '(?m)^    ([A-Za-z0-9_]+(?:::[A-Za-z0-9_]+)+)\s*$') } |
    ForEach-Object { $_.Groups[1].Value } |
    Sort-Object -Unique
$expected = Get-Content -LiteralPath $expectedPath |
    Where-Object { $_.Trim() } |
    ForEach-Object { $_.Trim() } |
    Sort-Object -Unique
$summaryMatches = [regex]::Matches(
    $text,
    'test result: (?:ok|FAILED)\.\s+(\d+) passed;\s+(\d+) failed;\s+(\d+) ignored;'
)
if ($summaryMatches.Count -eq 0) { throw 'Unable to locate Codex TUI test-count summaries.' }
$passed = ($summaryMatches | ForEach-Object { [int]$_.Groups[1].Value } | Measure-Object -Sum).Sum
$failed = ($summaryMatches | ForEach-Object { [int]$_.Groups[2].Value } | Measure-Object -Sum).Sum
$ignored = ($summaryMatches | ForEach-Object { [int]$_.Groups[3].Value } | Measure-Object -Sum).Sum
$unexpected = @($actual | Where-Object { $_ -notin $expected })
$transientTest = if ($unexpected.Count -eq 1) { $unexpected[0] } else { '' }
$transientBlockPattern = '(?s)---- ' + [regex]::Escape($transientTest) +
    ' stdout ----.*?An existing connection was forcibly closed by the remote host\. \(os error 10054\)'
if (
    $unexpected.Count -eq 1 -and
    $text -match $transientBlockPattern
) {
    Write-Output "Retrying the sole unexpected Windows transport failure: $transientTest"
    & cargo "+$Toolchain" test --locked --manifest-path $manifest -p codex-tui --lib $transientTest -- --exact
    if ($LASTEXITCODE -ne 0) { throw "The isolated Windows transport retry failed: $transientTest" }
    $actual = @($actual | Where-Object { $_ -ne $transientTest })
    $passed++
    $failed--
}
if ($passed -ne $ExpectedPassed -or $failed -ne 32 -or $ignored -ne 10) {
    throw "Codex TUI test counts changed: $passed passed, $failed failed, $ignored ignored; expected $ExpectedPassed/32/10."
}
$missing = @($expected | Where-Object { $_ -notin $actual })
$extra = @($actual | Where-Object { $_ -notin $expected })
if ($missing.Count -ne 0 -or $extra.Count -ne 0) {
    if ($missing.Count -ne 0) { Write-Error "Expected baseline failures absent: $($missing -join ', ')" }
    if ($extra.Count -ne 0) { Write-Error "New patched-suite failures: $($extra -join ', ')" }
    throw 'Patched Codex TUI failures differ from the independently recorded unpatched baseline.'
}
if ($testExit -eq 0) { throw 'Cargo reported success while an expected failure list was parsed.' }
Write-Output "Codex TUI suite matches the committed Windows failure baseline exactly ($passed passed, $failed known failures, $ignored ignored; no new failures)."
$global:LASTEXITCODE = 0
