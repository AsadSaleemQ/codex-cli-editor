[CmdletBinding()]
param([int64] $MaximumFileBytes = 1048576, [int64] $MaximumTotalBytes = 655360)

$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Push-Location $root
try {
    $files = @(
        git -c core.quotepath=false ls-files --cached --others --exclude-standard |
            ForEach-Object { ([string] $_).TrimEnd("`r") }
    )
    if ($LASTEXITCODE -ne 0 -or $files.Count -eq 0) { throw 'Unable to enumerate the publication candidate' }
    $findings = [Collections.Generic.List[object]]::new()
    $totalBytes = 0L
    foreach ($file in $files) {
        $item = Get-Item -Force -LiteralPath (Join-Path $root $file)
        $totalBytes += $item.Length
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            $findings.Add([pscustomobject]@{ Kind = 'reparse-point'; File = $file; Line = 0 })
        }
        if ($item.Length -gt $MaximumFileBytes) {
            $findings.Add([pscustomobject]@{ Kind = 'oversized'; File = $file; Line = 0 })
        }
        $bytes = [IO.File]::ReadAllBytes($item.FullName)
        if ($bytes -contains 0) {
            $findings.Add([pscustomobject]@{ Kind = 'binary'; File = $file; Line = 0 })
            continue
        }
        $lineNumber = 0
        foreach ($line in [IO.File]::ReadAllLines($item.FullName)) {
            $lineNumber++
            if ($line -match '-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----|\bgh[pousr]_[A-Za-z0-9]{20,}\b|\bgithub_pat_[A-Za-z0-9_]{30,}\b|\bsk-[A-Za-z0-9_-]{20,}\b|\bAKIA[0-9A-Z]{16}\b') {
                $findings.Add([pscustomobject]@{ Kind = 'credential-pattern'; File = $file; Line = $lineNumber })
            }
            if ($line -match '(?i)[A-Z]:\\(?:Users|Asad_VSCode)\\') {
                $findings.Add([pscustomobject]@{ Kind = 'absolute-local-path'; File = $file; Line = $lineNumber })
            }
            if ($line -match '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}') {
                $findings.Add([pscustomobject]@{ Kind = 'email-address'; File = $file; Line = $lineNumber })
            }
        }
    }
    $internalArtifactPatterns = @(
        '^CASE_STUDY\.md$',
        '^CODEX_.*RECONCILIATION.*\.md$',
        '^PATCH_REVIEW\.md$',
        '^(?:PLAN|REVIEW_HANDOFF|SESSION_HANDOFF)\.md$',
        '^docs/reviews/'
    )
    foreach ($file in $files) {
        if ($file -match '_[aA][iI](?:\.|$)') {
            $findings.Add([pscustomobject]@{ Kind = 'authorship-marker'; File = $file; Line = 0 })
        }
        if (@($internalArtifactPatterns | Where-Object { $file -match $_ }).Count -ne 0) {
            $findings.Add([pscustomobject]@{ Kind = 'internal-process-artifact'; File = $file; Line = 0 })
        }
    }
    if ($totalBytes -gt $MaximumTotalBytes) {
        $findings.Add([pscustomobject]@{ Kind = 'oversized-candidate'; File = '(all files)'; Line = 0 })
    }
    if ($findings.Count -ne 0) {
        $findings | Sort-Object Kind, File, Line | Format-Table -AutoSize | Out-Host
        throw "Publication boundary check found $($findings.Count) issue(s)"
    }
    Write-Output "Publication boundary check passed for $($files.Count) files ($totalBytes bytes)"
} finally {
    Pop-Location
}
