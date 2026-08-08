[CmdletBinding()]
param(
    [ValidateRange(1, 20)]
    [int]$Repeat = 1,

    [int]$Seed = 1505,

    [ValidateSet('A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q')]
    [string[]]$Scenario = @('A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $scriptRoot '..'))
$acceptanceTempBase = [System.IO.Path]::GetFullPath('D:\CodexBuild\textbooklens-p15t5-temp\acceptance')
$cargoTarget = [System.IO.Path]::GetFullPath('D:\CodexBuild\textbooklens-p15t5-target')
$runRoot = Join-Path $acceptanceTempBase ("run-{0}-{1}" -f $PID, [Guid]::NewGuid().ToString('N'))
$evidenceRoot = Join-Path $repositoryRoot ("test-results\acceptance-{0}-{1}" -f $Seed, [Guid]::NewGuid().ToString('N'))
$previewProcess = $null
$results = New-Object System.Collections.Generic.List[object]
$failures = 0

function New-CargoCheck {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][int]$ExpectedTests
    )
    return [pscustomobject]@{
        Name = $Name
        Arguments = $Arguments
        ExpectedTests = $ExpectedTests
    }
}

$commonIsolation = 'acceptance/scenario-isolation.spec.ts'
$manifest = [ordered]@{
    A = [pscustomobject]@{
        Name = 'book -> Key -> read; invalid validation retains safe state'
        Files = @($commonIsolation, 'onboarding-ai-services.spec.ts')
        Grep = '(A: isolated Temp|book-first onboarding keeps import)'
        ExpectedPlaywright = 2
        Cargo = @(
            (New-CargoCheck 'provider lifecycle compensation' @('test', '--manifest-path', 'src-tauri/Cargo.toml', '--all-features', '--test', 'provider_profile_lifecycle', '-j', '1') 3)
        )
    }
    B = [pscustomobject]@{
        Name = 'zh-CN / zh-TW / en persistence without content translation'
        Files = @($commonIsolation, 'localization-reader-shell.spec.ts', 'onboarding-ai-services.spec.ts')
        Grep = '(B: isolated Temp|switches all three application languages|three language entry points persist)'
        ExpectedPlaywright = 3
        Cargo = @()
    }
    C = [pscustomobject]@{
        Name = 'teaching instruction affects only new requests; preview writes no history'
        Files = @($commonIsolation, 'teaching-instructions.spec.ts')
        Grep = '(C: isolated Temp|scenario C)'
        ExpectedPlaywright = 6
        Cargo = @()
    }
    D = [pscustomobject]@{
        Name = 'direct reliable-text learning and atomic terminal truth'
        Files = @($commonIsolation, 'selection-regions.spec.ts')
        Grep = '(D: isolated Temp|D: immutable ordinary text)'
        ExpectedPlaywright = 2
        Cargo = @()
    }
    E = [pscustomobject]@{
        Name = 'three-format reliable region text with durable geometry'
        Files = @($commonIsolation, 'selection-regions.spec.ts')
        Grep = '(E: isolated Temp|E: reliable region text)'
        ExpectedPlaywright = 2
        Cargo = @(
            (New-CargoCheck 'PDF EPUB DOCX anchor restart' @('test', '--manifest-path', 'src-tauri/Cargo.toml', '--all-features', '--test', 'anchor_roundtrip', '-j', '1') 3)
        )
    }
    F = [pscustomobject]@{
        Name = 'visual-region consent, skip preference, stage, and handoff ordering'
        Files = @($commonIsolation, 'selection-regions.spec.ts')
        Grep = '(F: isolated Temp|F: visual region cancel)'
        ExpectedPlaywright = 2
        Cargo = @(
            (New-CargoCheck 'visual terminal persists no image bytes' @('test', '--manifest-path', 'src-tauri/Cargo.toml', '--all-features', '--test', 'learning_persistence', '-j', '1', 'visual_region_completion_preserves_null_selected_text_without_image_bytes', '--', '--exact') 1)
        )
    }
    G = [pscustomobject]@{
        Name = 'multiple floating panels and independent Stop'
        Files = @($commonIsolation, 'floating-learning-panels.spec.ts')
        Grep = '(G: isolated Temp|G: two real product requests)'
        ExpectedPlaywright = 2
        Cargo = @()
    }
    H = [pscustomobject]@{
        Name = 'hide collapse and route changes never cancel generation'
        Files = @($commonIsolation, 'floating-learning-panels.spec.ts')
        Grep = '(H: isolated Temp|H: collapse and hide)'
        ExpectedPlaywright = 2
        Cargo = @()
    }
    I = [pscustomobject]@{
        Name = 'restart history follow-up summary and atomic delete'
        Files = @($commonIsolation, 'floating-learning-panels.spec.ts')
        Grep = '(I: isolated Temp|I: restart restores only durable history)'
        ExpectedPlaywright = 2
        Cargo = @(
            (New-CargoCheck 'durable restart follow-up delete' @('test', '--manifest-path', 'src-tauri/Cargo.toml', '--all-features', '--test', 'learning_restart', '-j', '1') 2)
        )
    }
    J = [pscustomobject]@{
        Name = 'decline advanced indexing and keep local reading'
        Files = @($commonIsolation, 'ai-local-index.spec.ts')
        Grep = '(J: isolated Temp|J: rejecting the real ready-PDF entry)'
        ExpectedPlaywright = 2
        Cargo = @()
    }
    K = [pscustomobject]@{
        Name = 'local FTS preparation, abnormal pages, restart, Kimi region and Files durability'
        Files = @($commonIsolation, 'ai-local-index.spec.ts')
        Grep = '(K: isolated Temp|K: confirmed abnormal pages)'
        ExpectedPlaywright = 2
        Cargo = @(
            (New-CargoCheck 'page restart and offline FTS' @('test', '--manifest-path', 'src-tauri/Cargo.toml', '--all-features', '--test', 'ai_index_restart', '-j', '1', 'restart_recovers_every_transient_page_and_keeps_committed_fts_offline', '--', '--exact') 1),
            (New-CargoCheck 'Kimi CN international Files chat cleanup contracts' @('test', '--manifest-path', 'src-tauri/Cargo.toml', '--all-features', '--lib', '-j', '1', 'kimi_') 16),
            (New-CargoCheck 'image-only PDF page-section backfill' @('test', '--manifest-path', 'src-tauri/Cargo.toml', '--all-features', '--lib', '-j', '1', 'book_repository::tests::ensure_pdf_pages_backfills_missing_legacy_page_idempotently', '--', '--exact') 1)
        )
    }
    L = [pscustomobject]@{
        Name = 'partial failure, exact page retry, and aggregate correction'
        Files = @($commonIsolation, 'ai-local-index.spec.ts')
        Grep = '(L: isolated Temp|L: partial failure retries only)'
        ExpectedPlaywright = 2
        Cargo = @()
    }
    M = [pscustomobject]@{
        Name = 'manual correction precedence and explicit reanalysis conflict choice'
        Files = @($commonIsolation, 'acceptance/index-correction-conflict.spec.ts')
        Grep = 'M:'
        ExpectedPlaywright = 4
        Cargo = @(
            (New-CargoCheck 'correction CAS and conflict decisions' @('test', '--manifest-path', 'src-tauri/Cargo.toml', '--all-features', '--test', 'index_corrections', '-j', '1', 'unchanged_reanalysis_retains_correction_changed_target_conflicts_and_all_decisions_are_atomic', '--', '--exact') 1)
        )
    }
    N = [pscustomobject]@{
        Name = 'provenance distinction, quoteability, weighting, and book isolation'
        Files = @($commonIsolation, 'learning-overview-book-questions.spec.ts')
        Grep = '(N: isolated Temp|local overview stays offline)'
        ExpectedPlaywright = 2
        Cargo = @(
            (New-CargoCheck 'source distinction and book isolation' @('test', '--manifest-path', 'src-tauri/Cargo.toml', '--all-features', '--test', 'index_corrections', '-j', '1', 'retrieval_preserves_same_wording_sources_correction_audit_and_more_relevant_decoy_isolation', '--', '--exact') 1)
        )
    }
    O = [pscustomobject]@{
        Name = 'responsive fullscreen forced-colors reduced-motion automated boundary'
        Files = @($commonIsolation, 'accessibility-visual.spec.ts', 'localization-reader-shell.spec.ts')
        Grep = '(O: isolated Temp|keyboard, forced colors, reduced motion|F11 and toolbar fullscreen)'
        ExpectedPlaywright = 3
        Cargo = @()
    }
    P = [pscustomobject]@{
        Name = 'backup restore includes durable owned data and excludes credentials'
        Files = @($commonIsolation, 'local-data-privacy.spec.ts')
        Grep = '(P: isolated Temp|P: settings backup and restore)'
        ExpectedPlaywright = 2
        Cargo = @(
            (New-CargoCheck 'real synthetic TLBACKUP restore' @('test', '--manifest-path', 'src-tauri/Cargo.toml', '--all-features', '--test', 'local_data_privacy_checkpoint', '-j', '1', 'scenario_p_real_v1_backup_restores_three_formats_without_local_credentials', '--', '--exact') 1)
        )
    }
    Q = [pscustomobject]@{
        Name = 'delete one clear all credential cleanup and leak boundary'
        Files = @($commonIsolation, 'local-data-privacy.spec.ts')
        Grep = '(Q: isolated Temp|Q: busy failure and credential retry)'
        ExpectedPlaywright = 2
        Cargo = @(
            (New-CargoCheck 'clear retry and external-decoy preservation' @('test', '--manifest-path', 'src-tauri/Cargo.toml', '--all-features', '--test', 'local_data_privacy_checkpoint', '-j', '1', 'scenario_q_clear_retry_and_restart_remove_owned_data_but_preserve_external_decoys', '--', '--exact') 1),
            (New-CargoCheck 'one-book complete delete isolation' @('test', '--manifest-path', 'src-tauri/Cargo.toml', '--all-features', '--test', 'delete_book_complete', '-j', '1', 'complete_delete_removes_every_local_relation_and_owned_file_but_not_decoys_or_originals', '--', '--exact') 1)
        )
    }
}

function Assert-StrictChildPath {
    param(
        [Parameter(Mandatory = $true)][string]$Candidate,
        [Parameter(Mandatory = $true)][string]$Parent
    )
    $fullCandidate = [System.IO.Path]::GetFullPath($Candidate)
    $fullParent = [System.IO.Path]::GetFullPath($Parent).TrimEnd([System.IO.Path]::DirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
    if ($fullCandidate -eq $fullParent.TrimEnd([System.IO.Path]::DirectorySeparatorChar) -or -not $fullCandidate.StartsWith($fullParent, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing path outside the acceptance root."
    }
}

function Invoke-NativeCheck {
    param(
        [Parameter(Mandatory = $true)][string]$ScenarioId,
        [Parameter(Mandatory = $true)][int]$Attempt,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][int]$ExpectedTests
    )
    Write-Host ("[{0}] attempt {1}: {2} (expected tests: {3})" -f $ScenarioId, $Attempt, $Name, $ExpectedTests)
    # Windows PowerShell promotes redirected native stderr to ErrorRecord. Keep
    # warnings auditable without letting a successful command terminate the run.
    $previousErrorActionPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $output = & $FilePath @Arguments 2>&1
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    $output | ForEach-Object { Write-Host $_ }
    $countMatches = [regex]::Matches(($output | Out-String), '(?m)(\d+) passed')
    $actualTests = if ($countMatches.Count -gt 0) {
        [int]$countMatches[$countMatches.Count - 1].Groups[1].Value
    } else {
        -1
    }
    $countMatchesExpectation = $actualTests -eq $ExpectedTests
    $status = if ($exitCode -eq 0 -and $countMatchesExpectation) { 'PASS' } else { 'FAIL' }
    $script:results.Add([pscustomobject]@{
        scenario = $ScenarioId
        attempt = $Attempt
        check = $Name
        expectedTests = $ExpectedTests
        actualTests = $actualTests
        status = $status
        exitCode = $exitCode
    })
    if ($status -eq 'FAIL') {
        $script:failures += 1
        Write-Host ("[{0}] {1}: FAIL (exit {2}; actual tests {3}; expected {4})" -f $ScenarioId, $Name, $exitCode, $actualTests, $ExpectedTests) -ForegroundColor Red
        return
    }
    Write-Host ("[{0}] {1}: PASS ({2}/{3} tests)" -f $ScenarioId, $Name, $actualTests, $ExpectedTests) -ForegroundColor Green
}

function Get-ShuffledScenarios {
    param([string[]]$Values, [int]$ShuffleSeed)
    $items = New-Object System.Collections.Generic.List[string]
    foreach ($value in $Values) { $items.Add($value) }
    $random = New-Object System.Random($ShuffleSeed)
    for ($index = $items.Count - 1; $index -gt 0; $index -= 1) {
        $swap = $random.Next($index + 1)
        $current = $items[$index]
        $items[$index] = $items[$swap]
        $items[$swap] = $current
    }
    return $items.ToArray()
}

Push-Location $repositoryRoot
try {
    New-Item -ItemType Directory -Path $acceptanceTempBase -Force | Out-Null
    New-Item -ItemType Directory -Path $cargoTarget -Force | Out-Null
    New-Item -ItemType Directory -Path $runRoot | Out-Null
    Assert-StrictChildPath $runRoot $acceptanceTempBase

    $env:CARGO_TARGET_DIR = $cargoTarget
    $env:TEMP = 'D:\CodexBuild\textbooklens-p15t5-temp'
    $env:TMP = 'D:\CodexBuild\textbooklens-p15t5-temp'
    $env:CARGO_INCREMENTAL = '0'
    $env:CARGO_BUILD_JOBS = '1'
    $env:TEXTBOOKLENS_ACCEPTANCE_TEMP_BASE = $acceptanceTempBase

    $npm = (Get-Command 'npm.cmd' -ErrorAction Stop).Source
    $cargo = (Get-Command 'cargo.exe' -ErrorAction Stop).Source
    $node = (Get-Command 'node.exe' -ErrorAction Stop).Source

    Write-Host '[acceptance] building the frontend once for the isolated loopback preview'
    & $npm 'run' 'build'
    if ($LASTEXITCODE -ne 0) {
        throw "Acceptance prerequisite build failed with exit code $LASTEXITCODE."
    }

    $previewOut = Join-Path $runRoot 'preview.stdout.log'
    $previewError = Join-Path $runRoot 'preview.stderr.log'
    # Start-Process joins ArgumentList values on Windows PowerShell 5.1. Keep the
    # Vite entry relative to the explicit working directory so a repository path
    # containing spaces cannot be split into multiple Node arguments.
    $viteEntry = 'node_modules/vite/bin/vite.js'
    $previewProcess = Start-Process -FilePath $node -ArgumentList @($viteEntry, 'preview', '--host', '127.0.0.1', '--port', '1420') -WorkingDirectory $repositoryRoot -PassThru -WindowStyle Hidden -RedirectStandardOutput $previewOut -RedirectStandardError $previewError
    $ready = $false
    for ($probe = 0; $probe -lt 100 -and -not $ready; $probe += 1) {
        if ($previewProcess.HasExited) { break }
        try {
            $response = Invoke-WebRequest -UseBasicParsing -Uri 'http://127.0.0.1:1420/' -TimeoutSec 1
            $ready = $response.StatusCode -eq 200
        } catch {
            Start-Sleep -Milliseconds 200
        }
    }
    if (-not $ready) { throw 'The isolated loopback preview did not become ready.' }

    for ($attempt = 1; $attempt -le $Repeat; $attempt += 1) {
        $orderedScenarios = Get-ShuffledScenarios $Scenario ($Seed + $attempt - 1)
        Write-Host ("[acceptance] attempt {0}/{1}; seed {2}; order {3}" -f $attempt, $Repeat, ($Seed + $attempt - 1), ($orderedScenarios -join ','))
        foreach ($scenarioId in $orderedScenarios) {
            $entry = $manifest[$scenarioId]
            $scenarioRoot = Join-Path $runRoot ("attempt-{0}-{1}" -f $attempt, $scenarioId)
            Assert-StrictChildPath $scenarioRoot $runRoot
            New-Item -ItemType Directory -Path $scenarioRoot | Out-Null
            $env:TEXTBOOKLENS_ACCEPTANCE_SCENARIO = $scenarioId
            $env:TEXTBOOKLENS_ACCEPTANCE_APP_DATA = $scenarioRoot
            Write-Host ("[{0}] {1}" -f $scenarioId, $entry.Name) -ForegroundColor Cyan

            # Call the Playwright Node entry directly. Passing an alternation such
            # as "A:|isolation" through npx.cmd lets cmd.exe reinterpret `|` as a
            # pipeline before Playwright receives the grep expression.
            $playwrightArguments = @('node_modules/@playwright/test/cli.js', 'test') + [string[]]$entry.Files + @('--project=chromium', '--workers=1', '--reporter=list', '--grep', [string]$entry.Grep)
            Invoke-NativeCheck $scenarioId $attempt 'Playwright product flow + isolation contract' $node $playwrightArguments ([int]$entry.ExpectedPlaywright)

            foreach ($cargoCheck in $entry.Cargo) {
                Invoke-NativeCheck $scenarioId $attempt ([string]$cargoCheck.Name) $cargo ([string[]]$cargoCheck.Arguments) ([int]$cargoCheck.ExpectedTests)
            }

            $scenarioEvidence = Join-Path $scenarioRoot 'acceptance-evidence.json'
            if (Test-Path -LiteralPath $scenarioEvidence) {
                $evidence = Get-Content -Raw -LiteralPath $scenarioEvidence | ConvertFrom-Json
                $results.Add([pscustomobject]@{
                    scenario = $scenarioId
                    attempt = $attempt
                    check = 'isolated evidence file'
                    expectedTests = 1
                    status = 'PASS'
                    exitCode = 0
                })
                Write-Host ("[{0}] evidence: sources={1}; loopback={2}; external={3}; credentials={4}" -f $scenarioId, @($evidence.syntheticSources).Count, $evidence.loopbackProviderRequests, $evidence.externalProviderRequests, $evidence.credentialCount)
            } else {
                $results.Add([pscustomobject]@{
                    scenario = $scenarioId
                    attempt = $attempt
                    check = 'isolated evidence file'
                    expectedTests = 1
                    status = 'FAIL'
                    exitCode = 1
                })
                $failures += 1
                Write-Host ("[{0}] isolated evidence file: FAIL" -f $scenarioId) -ForegroundColor Red
            }

            Remove-Item -LiteralPath $scenarioRoot -Recurse -Force
        }
    }

    # Windows PowerShell 5.1 cannot reliably wrap a generic List[object] with
    # @(...). Copy it to a native PowerShell array before filtering/serializing.
    $resultArray = @()
    foreach ($result in $results) { $resultArray += $result }
    $summary = [ordered]@{
        suite = 'TextbookLens Phase 15 Task 5 A-Q'
        seed = $Seed
        repeat = $Repeat
        requestedScenarios = @($Scenario)
        checks = $resultArray.Count
        passed = @($resultArray | Where-Object { $_.status -eq 'PASS' }).Count
        failed = $failures
        status = if ($failures -eq 0) { 'PASS' } else { 'FAIL' }
        results = $resultArray
        manualGates = @(
            'Narrator/NVDA',
            'real Windows DPI and physical display transitions',
            'real maximized/fullscreen and contrast theme',
            'installer and clean Windows',
            'five real providers and credentials',
            'package signing and publishing'
        )
    }
    $summaryPath = Join-Path $evidenceRoot 'summary.json'
    New-Item -ItemType Directory -Path $evidenceRoot -Force | Out-Null
    $summary | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $summaryPath -Encoding UTF8
    Write-Host ("[acceptance] {0}: checks={1}; passed={2}; failed={3}; evidence=test-results/{4}/summary.json" -f $summary.status, $summary.checks, $summary.passed, $summary.failed, (Split-Path -Leaf $evidenceRoot))
}
catch {
    $failures += 1
    Write-Error $_
}
finally {
    Remove-Item Env:TEXTBOOKLENS_ACCEPTANCE_SCENARIO -ErrorAction SilentlyContinue
    Remove-Item Env:TEXTBOOKLENS_ACCEPTANCE_APP_DATA -ErrorAction SilentlyContinue
    if ($null -ne $previewProcess -and -not $previewProcess.HasExited) {
        Stop-Process -Id $previewProcess.Id -Force
        $previewProcess.WaitForExit()
    }
    if (Test-Path -LiteralPath $runRoot) {
        Assert-StrictChildPath $runRoot $acceptanceTempBase
        Remove-Item -LiteralPath $runRoot -Recurse -Force
    }
    Pop-Location
}

if ($failures -ne 0) { exit 1 }
exit 0
