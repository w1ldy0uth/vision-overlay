#requires -Version 5.1
[CmdletBinding()]
param([Parameter(Mandatory)][string]$CaptureDirectory, [string]$WarmupReplay)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$runDir = (Resolve-Path -LiteralPath $CaptureDirectory).Path
$etlPath = Join-Path $runDir 'capture.etl'
$pcapPath = Join-Path $runDir 'capture.pcapng'
if (!(Test-Path -LiteralPath $etlPath)) { throw "ETL file missing: $etlPath" }
if (!(Test-Path -LiteralPath $pcapPath)) {
    Write-Host 'Converting ETL to pcapng...'
    $conversion = & pktmon etl2pcap $etlPath --out $pcapPath 2>&1 | Out-String
    $conversion | Set-Content (Join-Path $runDir 'conversion.txt') -Encoding utf8
    if ($LASTEXITCODE -ne 0) { throw "ETL conversion failed: $conversion" }
}
$suffix = [guid]::NewGuid().ToString('N').Substring(0,8)
$outputPath = Join-Path $runDir "decode-$suffix.jsonl"
Write-Host 'Decoding exported packets offline...'
$decodeArguments = @('--keys',(Join-Path $projectRoot 'local-data/gi.json'),'--pcapng',$pcapPath,'--output',$outputPath)
if ($WarmupReplay) { $decodeArguments += @('--warmup-replay',(Resolve-Path -LiteralPath $WarmupReplay).Path) }
& (Join-Path $projectRoot 'target/debug/capture-probe.exe') @decodeArguments
if ($LASTEXITCODE -ne 0) { throw "Decode failed. ETL and pcapng were retained in $runDir" }
Write-Host "Capture and diagnostic output are ready: $runDir"
Write-Host "JSONL: $outputPath"
