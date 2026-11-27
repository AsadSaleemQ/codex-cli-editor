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
# This fixture launches real PowerShell and requires a readiness marker within ten seconds.
# Exercise its original deadline and shutdown assertions before the parallel test processes start.
$shellTest = 'app::tests::background_exit_tests::exit_interrupts_before_requesting_shutdown'
$isolated = @(& cargo "+$Toolchain" test --locked --manifest-path $manifest -p codex-tui --lib $shellTest -- --exact 2>&1)
if ($LASTEXITCODE -ne 0 -or ($isolated -join "`n") -notmatch 'test result: ok\. 1 passed; 0 failed;') {
    $isolated | Write-Output
    throw "Isolated shell shutdown test failed: $shellTest"
}
foreach ($line in $isolated) { $outputLines.Add($line.ToString()); Write-Output $line }
& cargo "+$Toolchain" test --locked --manifest-path $manifest -p codex-tui --no-fail-fast -- --skip $shellTest 2>&1 |
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
$planTest = 'app::tests::session_lifecycle_requests::new_session_tests::plan_implementation_reads_approved_plan_file_in_fresh_session'
$planFailure = [regex]::Match($text, '(?ms)^---- ' + [regex]::Escape($planTest) + ' stdout ----\r?\n(?<body>.*?)(?=^---- |^failures:|^test result:|\z)')
if ($planTest -in $actual -and $planFailure.Groups['body'].Value.Contains('app-server should emit a turn/completed event: Elapsed(())')) {
    Write-Output "Rechecking the app-server completion timeout in isolation: $planTest"
    $retry = @(& cargo "+$Toolchain" test --locked --manifest-path $manifest -p codex-tui --lib $planTest -- --exact 2>&1)
    $retryExit = $LASTEXITCODE
    $retry | Write-Output
    if ($retryExit -ne 0 -or ($retry -join "`n") -notmatch 'test result: ok\. 1 passed; 0 failed;') {
        throw "Isolated app-server completion test failed: $planTest"
    }
    $actual = @($actual | Where-Object { $_ -ne $planTest })
    $passed++
    $failed--
}
if ($AddedTests -eq 0) {
    $reference = @(Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\compatibility\codex-tui-windows-baseline-failures.txt'))
    if ($passed -ne 6220 -or $failed -ne $reference.Count -or $ignored -ne 7 -or
        @(Compare-Object $reference @($actual)).Count -ne 0) { throw 'Clean Windows TUI suite differs from the recorded 0.172.0 baseline.' }
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
