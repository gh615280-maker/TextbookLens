$ErrorActionPreference = 'Stop'
$required = @('node', 'npm', 'rustc', 'cargo')
$missing = $required | Where-Object { -not (Get-Command $_ -ErrorAction SilentlyContinue) }
if ($missing.Count -gt 0) { throw "Missing required tools: $($missing -join ', ')" }
$nodeMajor = [int]((node --version).TrimStart('v').Split('.')[0])
if ($nodeMajor -lt 22) { throw 'Node.js 22 or newer is required.' }
$rust = rustc --version
if ($rust -notmatch '^rustc 1\.(9[7-9]|[1-9][0-9]{2})\.') { throw "Rust 1.97+ is required; found $rust" }
Write-Host 'TextbookLens toolchain preflight passed.'
