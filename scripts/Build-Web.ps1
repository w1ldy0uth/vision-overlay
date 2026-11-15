#requires -Version 5.1
[CmdletBinding()]
param([switch]$PreparedCatalog)
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$npm=Get-Command npm.cmd -ErrorAction SilentlyContinue
Push-Location $root
try {
    if (!$PreparedCatalog) {
        $bundle=& (Join-Path $PSScriptRoot 'Prepare-Catalog.ps1')
        & cargo run --locked -p app-server --bin catalog-package -- --input $bundle --output local-data/build-input/catalog.payload.json --references-dir local-data/build-input/reference
        if ($LASTEXITCODE -ne 0) { throw 'Catalog preparation failed' }
    }
    if ($npm) {
        & $npm.Source --prefix web ci --no-audit --no-fund
        if ($LASTEXITCODE -ne 0) { throw 'Frontend dependency installation failed' }
        & $npm.Source --prefix web run build
    } else {
        $portable=Join-Path $root 'local-data/npm-runtime/package/bin/npm-cli.js'
        if (!(Test-Path -LiteralPath $portable)) { throw 'npm is required. Install npm alongside Node.js, then run this script again.' }
        & node $portable --prefix web ci --no-audit --no-fund
        if ($LASTEXITCODE -ne 0) { throw 'Frontend dependency installation failed' }
        & node $portable --prefix web run build
    }
    if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed' }
    Write-Host 'Web assets built. Rebuild vision-server to embed them: cargo build -p app-server --locked'
} finally { Pop-Location }
