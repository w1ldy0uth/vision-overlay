#requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet("auto","realtime","etw")][string]$CaptureBackend = "auto",
    [ValidateRange(1,86400)][int]$Duration,
    [ValidateRange(0,65535)][int]$Component = 0,
    [switch]$RecordRaw,
    [ValidateRange(1,16384)][int]$QueueCapacity = 8192,
    [ValidateRange(1,2048)][int]$RawBudgetMiB = 256,
    [ValidateRange(1,2048)][int]$RawSegmentMiB = 16,
    [ValidateRange(1024,65535)][int]$PanelPort = 18787,
    [switch]$NoPanel,
    [switch]$NoBrowser,
    [switch]$NoStopOnGameExit,
    [switch]$NoBackend,
    [switch]$TriggeredByGameLaunch,
    [guid]$ControlRunId,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Runtime.Paths.ps1')
$probe = Join-Path $binaryRoot 'capture-probe.exe'
$keys = Join-Path $dataRoot 'gi.json'
if (!$DryRun) {
    & (Join-Path $PSScriptRoot 'Test-CaptureEnvironment.ps1') -RequireCaptureApi -Backend $CaptureBackend | Out-Null
}
if ($RecordRaw -and $RawSegmentMiB -gt $RawBudgetMiB) { throw 'RawSegmentMiB must not exceed RawBudgetMiB.' }
$hasDeadline = $PSBoundParameters.ContainsKey('Duration')
$durationText = if ($hasDeadline) { "$Duration seconds (diagnostic deadline)" } else { 'until game exit or Ctrl+C (no deadline)' }
if ($NoStopOnGameExit -and !$hasDeadline) { $durationText = 'until Ctrl+C (no deadline)' }
if ($DryRun) {
    Write-Output "Plan: backend=$CaptureBackend, NIC sources (component=$Component), UDP 22101/22102, $durationText."
    Write-Output "Track materials, equipment, Mora, Primogems, talents, friendship and Adventure Rank/EXP; named timestamped events; JSONL in unique local-data/realtime-*; raw ring=$RecordRaw (budget $RawBudgetMiB MiB, segment $RawSegmentMiB MiB)."
    Write-Output "Game-process auto-stop enabled=$(!$NoStopOnGameExit); arms after GenshinImpact.exe is first detected; exit confirmation 3 seconds."
    Write-Output "Acquisition-to-decoder queue: $QueueCapacity packets (bounded)."
    Write-Output "Backend delivery descriptor enabled=$(!$NoBackend); API worker runs separately and does not block capture."
    Write-Output 'No global pktmon CLI filters are changed. No game process is launched.'
    Write-Output "Local live panel enabled=$(!$NoPanel), URL=http://127.0.0.1:$PanelPort/, open browser=$(!$NoBrowser)."
    return
}
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run this script in an administrator PowerShell terminal.'
}
foreach ($required in $probe,$keys) {
    if (!(Test-Path -LiteralPath $required -PathType Leaf)) { throw "Required file missing: $required" }
}
$runId = (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0,8)
$runDir = Join-Path $dataRoot "realtime-$runId"
New-Item -ItemType Directory -Path $runDir | Out-Null
$outputPath = Join-Path $runDir 'capture.jsonl'
if (!$NoBackend) {
    $descriptor = @{schema_version=1;capture_id=[guid]::NewGuid().ToString();log_file='capture.jsonl'}
    [IO.File]::WriteAllText((Join-Path $runDir 'delivery.json'),($descriptor | ConvertTo-Json),[Text.UTF8Encoding]::new($false))
}
$captureArguments = @('--keys',$keys,'--catalog',(Join-Path $dataRoot 'materials.ru.json'),'--avatar-catalog',(Join-Path $dataRoot 'avatars.ru.json'),'--achievement-catalog',(Join-Path $dataRoot 'achievements.ru.json'),'--track-materials','--transport','--queue-capacity',[string]$QueueCapacity,'--output',$outputPath)
if ($PSBoundParameters.ContainsKey('ControlRunId')) {
    $controlDir=Join-Path $dataRoot ('auto-capture/runs/'+$ControlRunId.ToString())
    if (!(Test-Path -LiteralPath $controlDir -PathType Container)) { throw 'Control run directory is missing.' }
    $captureArguments += @('--stop-file',(Join-Path $controlDir 'stop'),'--ready-file',(Join-Path $controlDir 'ready'))
    $session=@{schema_version=1;run_id=$ControlRunId.ToString();capture_id=$(if (!$NoBackend) {$descriptor.capture_id} else {$null});run_directory=(Split-Path $runDir -Leaf)}
    [IO.File]::WriteAllText((Join-Path $controlDir 'session.json'),($session | ConvertTo-Json -Compress),[Text.UTF8Encoding]::new($false))
}
if ($NoStopOnGameExit) { $captureArguments += '--no-stop-on-game-exit' }
if ($hasDeadline) { $captureArguments += @('--duration',[string]$Duration) }
if ($Component -gt 0) { $captureArguments += @('--component',[string]$Component) }
if ($RecordRaw) { $captureArguments += @('--record-ring',(Join-Path $runDir 'raw-ring'),'--record-max-mib',[string]$RawBudgetMiB,'--record-segment-mib',[string]$RawSegmentMiB) }
Write-Host "Files: $runDir"
if (!$NoBackend) { Write-Host 'Backend delivery enabled when vision-server is running; capture does not wait for the API.' }
if ($TriggeredByGameLaunch) { Write-Host 'Automatic capture: game launch detected. Waiting for the login snapshot; do not assume synchronization until it arrives.' }
else { Write-Host 'Fully close the game first. Wait for Realtime NIC capture active, then start the game.' }
Write-Host 'After Snapshot received: materials, equipment, Mora, Primogems, talents, friendship and Adventure Rank/EXP update immediately; console events include names and timestamps.'
if (!$NoStopOnGameExit) { Write-Host 'Auto-stop watches GenshinImpact.exe after first detection. Returning to login or switching accounts does not stop capture.' }
Write-Host "Capture runs $durationText. Ctrl+C stops capture. PAUSED means quantities are no longer current."
if ($RecordRaw) { Write-Host "Raw ring: $RawBudgetMiB MiB total / $RawSegmentMiB MiB segments. Old segments are evicted; capture continues." }
$panelProcess = $null
try {
    if (!$NoPanel) {
        $panel = Join-Path $binaryRoot 'live-view.exe'
        if (!(Test-Path -LiteralPath $panel -PathType Leaf)) { throw "Build live-view.exe first: cargo build --workspace --locked" }
        $panelArguments = @(('"' + $outputPath + '"'),'--catalog',('"' + (Join-Path $dataRoot 'materials.ru.json') + '"'),'--avatar-catalog',('"' + (Join-Path $dataRoot 'avatars.ru.json') + '"'), '--port',[string]$PanelPort,'--instance',$runId)
        $panelProcess = Start-Process -FilePath $panel -ArgumentList $panelArguments -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $runDir 'panel.stdout.log') -RedirectStandardError (Join-Path $runDir 'panel.stderr.log')
        $panelUrl = "http://127.0.0.1:$PanelPort/"
        $panelReady = $false
        for ($attempt = 0; $attempt -lt 30; $attempt++) {
            if ($panelProcess.HasExited) { throw "Live panel exited. See panel.stderr.log in $runDir" }
            try {
                $response = Invoke-RestMethod -Uri ($panelUrl + 'api/state') -TimeoutSec 1
                if ($response.instance -eq $runId) { $panelReady = $true; break }
            } catch { Start-Sleep -Milliseconds 100 }
        }
        if (!$panelReady -or $panelProcess.HasExited) { throw "Live panel failed to start: $runDir" }
        Write-Host "Live panel: $panelUrl"
        if (!$NoBrowser) { Start-Process $panelUrl }
    }
    $captureArguments += @('--capture-backend',$CaptureBackend)
    & $probe @captureArguments
    if ($LASTEXITCODE -ne 0) { throw "Realtime capture failed. Retained files: $runDir" }
} finally {
    if ($panelProcess -and !$panelProcess.HasExited) {
        Start-Sleep -Milliseconds 650
        Stop-Process -Id $panelProcess.Id
    }
}
$viewer = Join-Path $binaryRoot 'snapshot-view.exe'
$reportPath = Join-Path $runDir 'inventory-changes.html'
if (Test-Path -LiteralPath $viewer) { & $viewer $outputPath --catalog (Join-Path $dataRoot 'materials.ru.json') --html $reportPath
if ($LASTEXITCODE -ne 0) { Write-Warning "Report unavailable; capture JSONL retained: $outputPath" } }
Write-Host "Realtime capture finished. Files: $runDir"

# Report generation warnings must not turn a successful capture stop into a launcher failure.
exit 0
