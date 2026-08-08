[CmdletBinding()]
param(
    [string[]]$Stage = @('All'),
    [ValidateSet('A', 'B', 'O', 'P', 'Q')]
    [string[]]$AcceptanceScenario = @('A', 'B', 'O', 'P', 'Q'),
    [string]$BuildRoot = 'D:\CodexBuild\textbooklens-p15t7-preflight',
    [switch]$SkipArtifactBuild,
    [switch]$Offline
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$expectedNode = 'v24.18.1'
$expectedNpm = '11.16.0'
$expectedRust = '1.97.1'
$expectedTauriCli = '2.11.4'
$expectedTarget = 'x86_64-pc-windows-msvc'
$allowedStages = @('Toolchain', 'Frontend', 'Rust', 'Integrity', 'Acceptance', 'Bundle', 'All')
$Stage = @($Stage | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
if ($Stage.Count -eq 0 -or @($Stage | Where-Object { $_ -notin $allowedStages }).Count -gt 0) {
    throw 'Stage must contain only Toolchain, Frontend, Rust, Integrity, Acceptance, Bundle, or All.'
}

function Assert-TaskBuildRoot {
    param([Parameter(Mandatory = $true)][string]$Path)
    $fullPath = [System.IO.Path]::GetFullPath($Path).TrimEnd('\')
    $allowed = $fullPath -match '^D:\\CodexBuild\\textbooklens-p15t7(?:[-\\]|$)' -or
        ($env:CI -and $fullPath.StartsWith([System.IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\') + '\\', [System.StringComparison]::OrdinalIgnoreCase))
    if (-not $allowed) { throw 'BuildRoot must be a task-scoped D:\CodexBuild\textbooklens-p15t7* directory (or RUNNER_TEMP in CI).' }
    return $fullPath
}

function Invoke-Checked {
    param([Parameter(Mandatory = $true)][string]$Label, [Parameter(Mandatory = $true)][string]$FilePath, [string[]]$Arguments = @())
    Write-Host "[preflight] $Label"
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Label failed with exit code $LASTEXITCODE." }
}

function Test-Stage {
    param([string]$Name)
    return $Stage -contains 'All' -or $Stage -contains $Name
}

function Assert-ExactVersion {
    param([string]$Name, [string]$Actual, [string]$Expected)
    if ($Actual.Trim() -ne $Expected) { throw "$Name must be $Expected; found $($Actual.Trim())." }
}

try {
    $buildRoot = Assert-TaskBuildRoot $BuildRoot
    $required = @('git.exe', 'node.exe', 'npm.cmd', 'rustc.exe', 'cargo.exe')
    $missing = @($required | Where-Object { -not (Get-Command $_ -ErrorAction SilentlyContinue) })
    if ($missing.Count -gt 0) { throw "Missing required tools: $($missing -join ', ')" }
    & git.exe rev-parse --is-inside-work-tree *> $null
    if ($LASTEXITCODE -ne 0) { throw 'Preflight must run from a Git working tree so controlled files can be enumerated safely.' }
    $node = (Get-Command node.exe -ErrorAction Stop).Source
    $npm = (Get-Command npm.cmd -ErrorAction Stop).Source
    $cargo = (Get-Command cargo.exe -ErrorAction Stop).Source
    $rustc = (Get-Command rustc.exe -ErrorAction Stop).Source
    $env:CARGO_TARGET_DIR = Join-Path $buildRoot 'target'
    $env:TEMP = Join-Path $buildRoot 'temp'
    $env:TMP = $env:TEMP
    $env:CARGO_INCREMENTAL = '0'
    $env:CARGO_BUILD_JOBS = '1'
    if ($Offline) { $env:CARGO_NET_OFFLINE = 'true' }
    New-Item -ItemType Directory -Force -Path $env:CARGO_TARGET_DIR, $env:TEMP | Out-Null

    if (Test-Stage 'Toolchain') {
        Assert-ExactVersion 'Node.js' (& $node --version) $expectedNode
        Assert-ExactVersion 'npm' (& $npm --version) $expectedNpm
        $rustVersion = (& $rustc --version).Split(' ')[1]
        Assert-ExactVersion 'rustc' $rustVersion $expectedRust
        $cargoVersion = (& $cargo --version).Split(' ')[1]
        Assert-ExactVersion 'cargo' $cargoVersion $expectedRust
        $activeToolchain = (& rustup show active-toolchain).Split(' ')[0]
        if ($activeToolchain -ne "$expectedRust-$expectedTarget") { throw "Rust target toolchain must be $expectedRust-$expectedTarget; found $activeToolchain." }
        $tauriVersion = (& $node '-p' "require('./node_modules/@tauri-apps/cli/package.json').version")
        Assert-ExactVersion 'Tauri CLI' $tauriVersion $expectedTauriCli
        Write-Host "[preflight] toolchain locked: Node $expectedNode; npm $expectedNpm; Rust/Cargo $expectedRust; Tauri CLI $expectedTauriCli; target $expectedTarget"
    }

    Push-Location $repositoryRoot
    try {
        if (Test-Stage 'Frontend') {
            Invoke-Checked 'frontend format check' $npm @('run', 'format:check')
            Invoke-Checked 'frontend lint' $npm @('run', 'lint')
            Invoke-Checked 'frontend typecheck' $npm @('run', 'typecheck')
            Invoke-Checked 'frontend tests' $npm @('run', 'test')
            Invoke-Checked 'frontend build' $npm @('run', 'build')
        }
        if (Test-Stage 'Rust') {
            Invoke-Checked 'Rust format check' $cargo @('fmt', '--manifest-path', 'src-tauri/Cargo.toml', '--all', '--', '--check')
            Invoke-Checked 'Rust clippy' $cargo @('clippy', '--locked', '--manifest-path', 'src-tauri/Cargo.toml', '--all-targets', '--all-features', '-j', '1', '--', '-D', 'warnings')
            Invoke-Checked 'Rust tests' $cargo @('test', '--locked', '--manifest-path', 'src-tauri/Cargo.toml', '--all-features', '-j', '1')
        }
        if (Test-Stage 'Integrity') {
            Invoke-Checked 'sensitive-file check' $npm @('run', 'check:sensitive')
            Invoke-Checked 'npm license check' $npm @('run', 'check:licenses')
            Invoke-Checked 'fixture verification' $npm @('run', 'fixtures:verify')
            Invoke-Checked 'generated-file check' $npm @('run', 'check:generated')
        }
        if (Test-Stage 'Acceptance') {
            $files = @('e2e/acceptance/scenario-isolation.spec.ts', 'e2e/accessibility-visual.spec.ts', 'e2e/local-data-privacy.spec.ts')
            $grep = '(?:' + (($AcceptanceScenario | ForEach-Object { [regex]::Escape("${_}:") }) -join '|') + ')'
            # npm.cmd routes its arguments through cmd.exe, where `|` in the grep
            # expression becomes a pipeline. Invoke Playwright's local Node entry
            # directly so the exact A/B/O/P/Q alternation reaches Playwright.
            Invoke-Checked "Task 5 acceptance subset ($($AcceptanceScenario -join ','))" $node (@('node_modules/@playwright/test/cli.js', 'test') + $files + @('--project=chromium', '--grep', $grep, '--workers=1'))
        }
        if (Test-Stage 'Bundle' -and -not $SkipArtifactBuild) {
            Invoke-Checked 'Windows release bundle' $npm @('run', 'tauri', 'build')
            $bundleRoot = Join-Path $env:CARGO_TARGET_DIR 'release\bundle'
            Invoke-Checked 'release artifact scan' $node @('scripts/audit-release-artifacts.mjs', '--no-defaults', '--root', $bundleRoot, '--root', (Join-Path $repositoryRoot 'src-tauri\resources'), '--root', (Join-Path $repositoryRoot 'src-tauri\migrations'), '--file', (Join-Path $repositoryRoot 'LICENSES.md'), '--file', (Join-Path $repositoryRoot 'THIRD_PARTY_NOTICES.md'))
        }
    } finally {
        Pop-Location
    }
    Write-Host '[preflight] PASS'
    exit 0
} catch {
    Write-Error "[preflight] FAIL: $($_.Exception.Message)"
    exit 1
}
