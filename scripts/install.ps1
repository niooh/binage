$ErrorActionPreference = 'Stop'

# Locate the repository root from this script's location.
$repoRoot = Split-Path -Parent $PSScriptRoot
$cargoToml = Join-Path $repoRoot 'Cargo.toml'

if (-not (Test-Path -LiteralPath $cargoToml -PathType Leaf)) {
    throw "Cannot find Cargo.toml in repository root: $repoRoot"
}

# Check Cargo.
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'Cargo was not found. Install Rust before running this script.'
}

Push-Location $repoRoot

try {
    # Build the release executable.
    & cargo build --release --bin binage --quiet

    if ($LASTEXITCODE -ne 0) {
        throw "Cargo build failed with exit code $LASTEXITCODE."
    }

    $binageExe = Join-Path $repoRoot 'target\release\binage.exe'

    if (-not (Test-Path -LiteralPath $binageExe -PathType Leaf)) {
        throw "Build succeeded, but executable was not found: $binageExe"
    }

    # Run the existing initialization logic.
    & $binageExe init

    if ($LASTEXITCODE -ne 0) {
        throw "binage init failed with exit code $LASTEXITCODE."
    }

    # Resolve the configured installation root.
    $configBase = if ($env:APPDATA) {
        $env:APPDATA
    } else {
        $env:LOCALAPPDATA
    }

    $rootPointer = Join-Path $configBase 'binage\root.txt'

    if (Test-Path -LiteralPath $rootPointer -PathType Leaf) {
        $installRoot = (Get-Content -LiteralPath $rootPointer -Raw).Trim()
    } else {
        $installRoot = Join-Path $env:LOCALAPPDATA 'binage'
    }

    if (-not $installRoot) {
        throw 'Cannot determine the binage installation root.'
    }

    $binDir = Join-Path $installRoot 'bin'

    if (-not (Test-Path -LiteralPath $binDir -PathType Container)) {
        throw "Installation directory does not exist: $binDir"
    }

    # Update the current PowerShell process.
    if (($env:Path -split ';') -notcontains $binDir) {
        $env:Path = "$binDir;$env:Path"
    }

    Write-Host 'binage is ready in the current PowerShell session.'
}
finally {
    Pop-Location
}
