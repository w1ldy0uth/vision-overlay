#requires -Version 5.1
[CmdletBinding()]
param(
    [ValidatePattern('^(nics|[1-9][0-9]*)$')][string]$Component = 'nics',
    [ValidateRange(15,600)][int]$Duration = 180,
    [ValidateRange(64,1024)][int]$MaxFileMiB = 256,
    [switch]$ListComponents,
    [string]$WarmupReplay,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ($WarmupReplay) { $WarmupReplay = (Resolve-Path -LiteralPath $WarmupReplay).Path }
if ($DryRun) {
    Write-Output "Plan: check idle pktmon and empty filters; create unique local-data/etl-* directory."
    Write-Output "pktmon start --capture --comp $Component --type flow --pkt-size 0 --file-size $MaxFileMiB --log-mode circular --file-name <unique-directory>/capture.etl"
    Write-Output "Stop on Enter or after $Duration seconds; restore empty filters only if unchanged since setup; convert and decode offline."
    return
}
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run this script in an administrator PowerShell terminal.'
}
function Invoke-Pktmon([string[]]$Arguments) {
    $result = & pktmon @Arguments 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "pktmon $($Arguments -join ' ') failed: $result" }
    return $result
}
function Test-EmptyPktmonFilters([string]$Text) {
    $normalized = ($Text -replace '\s+', ' ').Trim()
    return $normalized -match '^(?i:Packet Filters:\s*None|No (?:packet )?filters(?: (?:defined|have been added))?|Нет фильтров|Фильтры отсутствуют)\.?$'
}
if ($ListComponents) { Invoke-Pktmon -Arguments @('list','--all'); return }
foreach ($required in @('target/debug/capture-probe.exe','local-data/gi.json')) {
    if (!(Test-Path -LiteralPath (Join-Path $projectRoot $required))) { throw "Required file missing: $required" }
}
$status = Invoke-Pktmon -Arguments @('status')
if (Get-Process -Name capture-probe -ErrorAction SilentlyContinue) { throw 'Another capture-probe process is running; finish it first.' }
if ($status -notmatch '(?i)not running|не запущен|не выполняется|(?m)^\s*(Status|Состояние)\s*:\s*(Stopped|Остановлен[ао]?)\s*$') {
    throw "Pktmon is active or its idle state was not recognized. No capture settings changed. Status: $status"
}
$filters = Invoke-Pktmon -Arguments @('filter','list')
if (!(Test-EmptyPktmonFilters -Text $filters)) {
    throw "Existing filters or an unrecognized filter list were found. They were left unchanged: $filters"
}
$runId = 'etl-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0,8)
$runDir = Join-Path $projectRoot "local-data/$runId"
New-Item -ItemType Directory -Path $runDir | Out-Null
$etlPath = Join-Path $runDir 'capture.etl'
$status | Set-Content (Join-Path $runDir 'status-before.txt') -Encoding utf8
$filters | Set-Content (Join-Path $runDir 'filters-before.txt') -Encoding utf8
@{ component=$Component; duration_seconds=$Duration; max_file_mib=$MaxFileMiB; mode='circular'; full_packets=$true; started_at=[DateTime]::UtcNow.ToString('o') } |
    ConvertTo-Json | Set-Content (Join-Path $runDir 'capture-config.json') -Encoding utf8
$ownedFilters = [Collections.Generic.List[string]]::new()
$ownedFilterSnapshot = $null
$started = $false
try {
    foreach ($port in 22101,22102) {
        $name = "VisionOverlay-$runId-$port"
        Invoke-Pktmon -Arguments @('filter','add',$name,'-t','UDP','-p',[string]$port) | Out-Null
        $ownedFilters.Add($name)
        $ownedFilterSnapshot = (Invoke-Pktmon -Arguments @('filter','list')).Trim()
    }
    $startOutput = Invoke-Pktmon -Arguments @('start','--capture','--comp',$Component,'--type','flow','--pkt-size','0','--file-name',$etlPath,'--file-size',[string]$MaxFileMiB,'--log-mode','circular')
    $started = $true
    $startOutput | Set-Content (Join-Path $runDir 'capture-start.txt') -Encoding utf8
    Write-Host "ETL capture active ($Component). Enter the game now."
    Write-Host "After loading: wait 10 seconds, pick up one item, wait another 10 seconds."
    Write-Host "Return here and press Enter to stop. Auto-stop: $Duration seconds."
    Write-Host "Files: $runDir"
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while ($timer.Elapsed.TotalSeconds -lt $Duration) {
        if (![Console]::IsInputRedirected -and [Console]::KeyAvailable) {
            if ([Console]::ReadKey($true).Key -eq [ConsoleKey]::Enter) { break }
        }
        Start-Sleep -Milliseconds 200
    }
} finally {
    if ($started) {
        try {
            $stopOutput = Invoke-Pktmon -Arguments @('stop')
            $started = $false
            $stopOutput | Set-Content (Join-Path $runDir 'capture-stop.txt') -Encoding utf8
        } catch { Write-Warning "Could not stop the capture: $_. Files: $runDir" }
    }
    if (!$started -and $ownedFilters.Count -gt 0) {
        try {
            $currentFilters = (Invoke-Pktmon -Arguments @('filter','list')).Trim()
            if ($ownedFilterSnapshot -and $currentFilters -ceq $ownedFilterSnapshot) {
                # This Windows CLI supports only remove-all. Initial filters were empty,
                # and the full list must still match what this run created.
                Invoke-Pktmon -Arguments @('filter','remove') | Out-Null
            } else { Write-Warning 'Filters changed during capture; cleanup skipped to preserve other filters.' }
        } catch { Write-Warning "Filter cleanup failed: $_" }
    }
}
if ($started) { throw 'Capture stop was not confirmed; import was skipped.' }
& (Join-Path $PSScriptRoot 'Import-EtlCapture.ps1') -CaptureDirectory $runDir -WarmupReplay $WarmupReplay
