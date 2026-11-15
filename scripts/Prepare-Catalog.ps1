#requires -Version 5.1
[CmdletBinding()]
param([string]$CatalogBundle)
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ($CatalogBundle) {
    $resolved=(Resolve-Path -LiteralPath $CatalogBundle).Path
    Write-Output $resolved
    return
}
$inputRoot=Join-Path $root 'packaging/catalog'
$manifest=Get-Content -LiteralPath (Join-Path $inputRoot 'source.json') -Raw | ConvertFrom-Json
$archive=Join-Path $inputRoot 'bootstrap.visioncatalog.gz'
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $manifest.compressed_sha256) { throw 'Pinned bootstrap archive hash mismatch' }
$outputRoot=Join-Path $root 'local-data/build-input'
New-Item -ItemType Directory -Path $outputRoot -Force | Out-Null
$output=Join-Path $outputRoot 'bootstrap.visioncatalog'
$source=[IO.File]::OpenRead($archive)
$gzip=[IO.Compression.GzipStream]::new($source,[IO.Compression.CompressionMode]::Decompress)
$target=[IO.File]::Create($output)
try { $gzip.CopyTo($target) } finally { $target.Dispose();$gzip.Dispose();$source.Dispose() }
if ((Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash.ToLowerInvariant() -ne $manifest.bundle_sha256) { throw 'Pinned bootstrap bundle hash mismatch' }
Write-Output $output
