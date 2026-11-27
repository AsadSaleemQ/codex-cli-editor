[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $UpstreamRoot,
    [string] $Toolchain = '1.99.1-x86_64-pc-windows-msvc',
    [ValidateRange(0, 100)] [int] $AddedTests = 0
)

$ErrorActionPreference = 'Stop'
$env:RUST_MIN_STACK = '16777216'
$env:INSTA_WORKSPACE_ROOT = [IO.Path]::GetFullPath((Join-Path $UpstreamRoot 'codex-rs'))
$env:INSTA_UPDATE = 'no'
$v8 = & (Join-Path $PSScriptRoot 'prepare_rusty_v8.ps1') -UpstreamRoot $UpstreamRoot -DestinationRoot (Join-Path $UpstreamRoot '..\rusty-v8')
$env:RUSTY_V8_ARCHIVE = $v8.ArchivePath
$env:RUSTY_V8_SRC_BINDING_PATH = $v8.BindingPath
$manifest = Join-Path $UpstreamRoot 'codex-rs\Cargo.toml'
$expectedPath = Join-Path $UpstreamRoot '.codex-cli-editor-test-baseline.json'
$commit = (& git -C $UpstreamRoot rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $commit -ne '3d86e43b075adeba2193ec5c22ac92f55afd82e6') { throw 'Unexpected upstream commit for the Windows test baseline.' }
if ($AddedTests -eq 0) {
    & git -C $UpstreamRoot diff --quiet HEAD -- codex-rs/tui
    if ($LASTEXITCODE -ne 0) { throw 'Measure the baseline before applying the TUI patch.' }
}
if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) { throw "Missing upstream manifest: $manifest" }
if ($AddedTests -gt 0 -and -not (Test-Path -LiteralPath $expectedPath -PathType Leaf)) { throw "Missing failure baseline: $expectedPath" }

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
if ($testExit -ne 0 -and ($failureBlocks.Count -eq 0 -or $text -match '(?m)^error: (could not compile|doctest failed|process did not exit successfully)')) {
    throw 'Cargo failed to execute the complete Windows test suite.'
}
$actual = $failureBlocks |
    ForEach-Object { [regex]::Matches($_.Groups['body'].Value, '(?m)^    ([A-Za-z0-9_]+(?:::[A-Za-z0-9_]+)+)\s*$') } |
    ForEach-Object { $_.Groups[1].Value } |
    Sort-Object -Unique
$summaryMatches = [regex]::Matches(
    $text,
    'test result: (?:ok|FAILED)\.\s+(\d+) passed;\s+(\d+) failed;\s+(\d+) ignored;'
)
if ($summaryMatches.Count -eq 0) { throw 'Unable to locate Codex TUI test-count summaries.' }
$passed = ($summaryMatches | ForEach-Object { [int]$_.Groups[1].Value } | Measure-Object -Sum).Sum
$failed = ($summaryMatches | ForEach-Object { [int]$_.Groups[2].Value } | Measure-Object -Sum).Sum
$ignored = ($summaryMatches | ForEach-Object { [int]$_.Groups[3].Value } | Measure-Object -Sum).Sum
if ($AddedTests -eq 0) {
    if ($passed -lt 4000 -or $failed -ne @($actual | Where-Object { $_ }).Count) { throw 'Incomplete clean Windows TUI test inventory.' }
    [ordered]@{ commit = $commit; passed = $passed; failed = $failed; ignored = $ignored; failures = @($actual | Where-Object { $_ }) } |
        ConvertTo-Json | Set-Content -LiteralPath $expectedPath
    Write-Output "Measured clean Codex Windows baseline ($passed passed, $failed failed, $ignored ignored)."
    $global:LASTEXITCODE = 0
    return
}
$baseline = Get-Content -LiteralPath $expectedPath -Raw | ConvertFrom-Json
if ($baseline.commit -ne $commit) { throw 'Baseline commit mismatch.' }
$expected = @($baseline.failures)
$ExpectedPassed = $baseline.passed + $AddedTests
$unexpected = @($actual | Where-Object { $_ -notin $expected })
$retryableTransportFailures = @($unexpected | Where-Object {
    $pattern = '(?s)---- ' + [regex]::Escape($_) +
        ' stdout ----.*?An existing connection was forcibly closed by the remote host\. \(os error 10054\)'
    $text -match $pattern
})
if (
    $unexpected.Count -ge 1 -and
    $unexpected.Count -le 4 -and
    $retryableTransportFailures.Count -eq $unexpected.Count
) {
    foreach ($transientTest in $retryableTransportFailures) {
        $recovered = $false
        for ($attempt = 1; $attempt -le 3; $attempt++) {
            Write-Output "Retrying unexpected Windows transport failure in isolation ($attempt/3): $transientTest"
            $retryLines = [Collections.Generic.List[string]]::new()
            $ErrorActionPreference = 'Continue'
            & cargo "+$Toolchain" test --locked --manifest-path $manifest -p codex-tui --lib $transientTest -- --exact 2>&1 |
                ForEach-Object {
                    $line = $_.ToString()
                    $retryLines.Add($line)
                    Write-Output $line
                }
            $retryExit = $LASTEXITCODE
            $ErrorActionPreference = 'Stop'
            if ($retryExit -eq 0) {
                $recovered = $true
                break
            }
            $retryText = $retryLines -join "`n"
            $retryFailurePattern = '(?s)---- ' + [regex]::Escape($transientTest) +
                ' stdout ----.*?An existing connection was forcibly closed by the remote host\. \(os error 10054\)'
            if ($retryText -notmatch $retryFailurePattern) {
                throw "The isolated retry failed for a reason other than the exact Windows transport reset: $transientTest"
            }
        }
        if (-not $recovered) { throw "The isolated Windows transport retry failed three times: $transientTest" }
    }
    $actual = @($actual | Where-Object { $_ -notin $retryableTransportFailures })
    $passed += $retryableTransportFailures.Count
    $failed -= $retryableTransportFailures.Count
}
if ($passed -ne $ExpectedPassed -or $failed -ne $baseline.failed -or $ignored -ne $baseline.ignored) {
    throw "Codex TUI test counts changed: $passed passed, $failed failed, $ignored ignored; expected $ExpectedPassed/$($baseline.failed)/$($baseline.ignored)."
}
$missing = @($expected | Where-Object { $_ -notin $actual })
$extra = @($actual | Where-Object { $_ -notin $expected })
if ($missing.Count -ne 0 -or $extra.Count -ne 0) {
    if ($missing.Count -ne 0) { Write-Error "Expected baseline failures absent: $($missing -join ', ')" }
    if ($extra.Count -ne 0) { Write-Error "New patched-suite failures: $($extra -join ', ')" }
    throw 'Patched Codex TUI failures differ from the independently recorded unpatched baseline.'
}
if (($testExit -eq 0) -ne ($failed -eq 0 -and $retryableTransportFailures.Count -eq 0)) { throw 'Cargo exit status differs from parsed test results.' }
Write-Output "Codex TUI suite matches the clean Windows failure baseline exactly ($passed passed, $failed known failures, $ignored ignored; no new failures)."
$global:LASTEXITCODE = 0
