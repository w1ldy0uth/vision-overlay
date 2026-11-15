#requires -Version 5.1
[CmdletBinding()]
param([switch]$DryRun)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'Runtime.Paths.ps1')
$stateDir=Join-Path $dataRoot 'auto-capture'
$disabledFile=Join-Path $stateDir 'disabled'
if ($DryRun) {
    Write-Output 'Watch GenshinImpact.exe every 250 ms; automatic and manual capture share one owner.'
    Write-Output 'Control v1 accepts only fixed start/stop requests bound to this helper and capture run; no shell commands.'
    Write-Output 'Stop writes a per-run marker; probe drains and finalizes. Disabling automatic starts keeps manual control available.'
    return
}
$identity=[Security.Principal.WindowsIdentity]::GetCurrent()
$principal=[Security.Principal.WindowsPrincipal]::new($identity)
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Capture helper requires an administrator token.' }
foreach ($required in (Join-Path $binaryRoot 'capture-probe.exe'),(Join-Path $dataRoot 'gi.json'),(Join-Path $PSScriptRoot 'Start-RealtimeCapture.ps1')) {
    if (!(Test-Path -LiteralPath $required -PathType Leaf)) { throw "Required file missing: $required" }
}
New-Item -ItemType Directory -Path $stateDir -Force | Out-Null
. (Join-Path $PSScriptRoot 'AutoCapture.State.ps1')
$sha=[Security.Cryptography.SHA256]::Create()
try { $suffix=([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($projectRoot.ToLowerInvariant())))).Replace('-','').Substring(0,16) } finally { $sha.Dispose() }
$created=$false
$guard=[Threading.Mutex]::new($false,"Local\VisionOverlayAutoCapture-$suffix",[ref]$created)
if (!$created) { $guard.Dispose(); throw 'Capture helper already running for this checkout.' }
$instance=[guid]::NewGuid().ToString()
$policy=@{Initialized=$false;Attempted=$null}
$child=$null;$runId=$null;$lastRunId=$null;$stopping=$false;$lastExitCode=$null;$lastCommand=$null
$sessionId=(Get-Process -Id $PID).SessionId
$statusFile=Join-Path $stateDir 'status.json'
$processed=@{}
$commandFile=Join-Path $stateDir 'command.json'
$lastStatus='';$lastHeartbeat=[DateTime]::MinValue;$lastPublishError=[DateTime]::MinValue
function Set-ControlResult($Command,[string]$Outcome,[string]$Code) {
    $script:lastCommand=@{request_id=[string]$Command.request_id;action=[string]$Command.action;target_run_id=$Command.target_run_id;outcome=$Outcome;code=$Code;run_id=$script:runId;updated_at=[DateTime]::UtcNow.ToString('o')}
}
function Start-OwnedCapture([bool]$Automatic) {
    $script:lastExitCode=$null;$script:stopping=$false
    $script:runId=[guid]::NewGuid().ToString();$script:lastRunId=$script:runId
    New-Item -ItemType Directory -Path (Join-Path $stateDir ('runs/'+$script:runId)) | Out-Null
    $launchId=(Get-Date -Format 'yyyyMMdd-HHmmss')+'-'+$script:runId.Substring(0,8)
    $scriptPath=Join-Path $PSScriptRoot 'Start-RealtimeCapture.ps1'
    $arguments=@('-NoProfile','-ExecutionPolicy','RemoteSigned','-File',('"'+$scriptPath+'"'),'-RecordRaw','-NoPanel','-NoBrowser','-ControlRunId',$script:runId)
    if ($Automatic) { $arguments+='-TriggeredByGameLaunch' }
    try {
        $script:child=Start-Process -FilePath (Join-Path $env:WINDIR 'System32/WindowsPowerShell/v1.0/powershell.exe') -ArgumentList $arguments -WorkingDirectory $projectRoot -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $stateDir "$launchId.stdout.log") -RedirectStandardError (Join-Path $stateDir "$launchId.stderr.log")
        # Cache the process handle while alive; PS5.1 may otherwise report a null ExitCode.
        $null=$script:child.Handle
    } catch { $script:lastExitCode=-1;$script:runId=$null;throw }
}
function Publish-ControlStatus([string]$State,[string]$GameKey,[bool]$External) {
    $session=$null
    if ($runId) { $sessionPath=Join-Path $stateDir ('runs/'+$runId+'/session.json');if (Test-Path -LiteralPath $sessionPath) {try {$session=[IO.File]::ReadAllText($sessionPath) | ConvertFrom-Json} catch {}} }
    $payload=@{schema_version=1;control_version=1;watcher_instance=$instance;state=$State;watcher_pid=$PID;game_key=$GameKey;updated_at=[DateTime]::UtcNow.ToString('o');capture_host_pid=$(if($child){$child.Id}else{$null});capture_run_id=$runId;last_run_id=$lastRunId;capture_id=$session.capture_id;run_directory=$session.run_directory;external_capture=$External;last_exit_code=$lastExitCode;last_command=$lastCommand}
    Write-AutoCaptureJsonFile $statusFile ($payload | ConvertTo-Json -Compress -Depth 5)
}
try {
    while ($true) {
        $disabled=Test-Path -LiteralPath $disabledFile
        if ($child -and $child.HasExited) {
            $child.WaitForExit();$lastExitCode=$(if($null -eq $child.ExitCode){-1}else{[int]$child.ExitCode})
            if ($lastCommand -and $lastCommand.run_id -eq $runId -and $lastCommand.outcome -eq 'accepted') {
                $lastCommand.outcome=$(if($lastExitCode -eq 0){'completed'}else{'failed'});$lastCommand.code=$(if($lastExitCode -eq 0){'capture_finished'}else{'capture_failed'})
            }
            $child.Dispose();$child=$null;$runId=$null;$stopping=$false
        }
        $gameKey=$null;$known=$true;$otherCapture=$false;$processes=@()
        try {
            $processes=@(Get-Process -ErrorAction Stop)
            $games=@($processes | Where-Object {$_.ProcessName -eq 'GenshinImpact' -and $_.SessionId -eq $sessionId})
            if ($games.Count) { $game=$games | Sort-Object Id | Select-Object -First 1;$gameKey=('{0}:{1}' -f $game.Id,$game.StartTime.ToUniversalTime().Ticks) }
            $otherCapture=@($processes | Where-Object {$_.ProcessName -eq 'capture-probe' -and $_.SessionId -eq $sessionId}).Count -gt 0
        } catch {$known=$false} finally {foreach($observed in $processes){$observed.Dispose()}}
        if (Test-Path -LiteralPath $commandFile) {
            $cmd=$null
            try {
                if ((Get-Item -LiteralPath $commandFile).Length -gt 4096) { throw 'Command exceeds size limit' }
                $cmd=[IO.File]::ReadAllText($commandFile) | ConvertFrom-Json
                $validation=Test-AutoCaptureCommand $cmd $instance ([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
                if ($validation -ne 'valid') {Set-ControlResult $cmd 'rejected' $validation}
                elseif ($processed.ContainsKey([string]$cmd.request_id)) { $lastCommand=$processed[[string]$cmd.request_id].result }
                elseif ($cmd.action -eq 'start') {
                    if (!$known) {Set-ControlResult $cmd 'rejected' 'observation_failed'}
                    elseif ($child -or $otherCapture) {Set-ControlResult $cmd 'rejected' 'already_running'}
                    else {
                        $policy.Initialized=$true;$policy.Attempted=$gameKey
                        try {Start-OwnedCapture $false;Set-ControlResult $cmd 'accepted' 'starting'}
                        catch {Set-ControlResult $cmd 'failed' 'launch_failed';Add-Content -LiteralPath (Join-Path $stateDir 'watcher.log') -Value $_.Exception.Message}
                    }
                } else {
                    if (!$child -or $cmd.target_run_id -ne $runId) {Set-ControlResult $cmd 'rejected' 'run_changed'}
                    else {
                        [IO.File]::WriteAllText((Join-Path $stateDir ('runs/'+$runId+'/stop')),'stop',[Text.UTF8Encoding]::new($false))
                        $stopping=$true
                        if($gameKey){$policy.Attempted=$gameKey}
                        Set-ControlResult $cmd 'accepted' 'stopping'
                    }
                }
                if ($validation -eq 'valid') {
                    $processed[[string]$cmd.request_id]=@{result=$lastCommand;expiry=[long]$cmd.expires_unix_ms}
                    foreach ($key in @($processed.Keys)) {if($processed[$key].expiry -lt [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()){$processed.Remove($key)}}
                }
            } catch {Set-ControlResult $cmd 'failed' 'invalid_command';Add-Content -LiteralPath (Join-Path $stateDir 'watcher.log') -Value $_.Exception.Message}
            finally {Remove-Item -LiteralPath $commandFile -ErrorAction Stop}
        }
        $decision=Get-AutoCaptureDecision $policy $gameKey $known ($otherCapture -or [bool]$child) $disabled
        if ($decision -eq 'launch') {
            try {Start-OwnedCapture $true} catch {Add-Content -LiteralPath (Join-Path $stateDir 'watcher.log') -Value $_.Exception.Message}
        }
        $ready=$runId -and (Test-Path -LiteralPath (Join-Path $stateDir ('runs/'+$runId+'/ready')))
        $state=if($child){if($stopping){'stopping'}elseif($ready){'ready'}else{'starting'}}elseif($lastExitCode -and $gameKey){'capture_failed'}elseif($disabled){'idle'}else{$decision}
        if($ready -and $lastCommand -and $lastCommand.action -eq 'start' -and $lastCommand.run_id -eq $runId -and $lastCommand.outcome -eq 'accepted'){$lastCommand.outcome='completed';$lastCommand.code='ready'}
        $signature=$state+'|'+$runId+'|'+$gameKey+'|'+[string]$lastCommand.request_id+'|'+[string]$lastCommand.outcome
        if($signature -ne $lastStatus -or ([DateTime]::UtcNow-$lastHeartbeat).TotalSeconds -ge 2){
            try {
                Publish-ControlStatus $state $gameKey ($otherCapture -and !$child)
                $lastStatus=$signature;$lastHeartbeat=[DateTime]::UtcNow
            } catch {
                if(([DateTime]::UtcNow-$lastPublishError).TotalSeconds -ge 30){
                    Add-Content -LiteralPath (Join-Path $stateDir 'watcher.log') -Value (([DateTime]::UtcNow.ToString('o'))+' Status publication delayed: '+$_.Exception.Message) -ErrorAction SilentlyContinue
                    $lastPublishError=[DateTime]::UtcNow
                }
            }
        }
        Start-Sleep -Milliseconds 250
    }
} catch {
    Add-Content -LiteralPath (Join-Path $stateDir 'watcher.log') -Value (([DateTime]::UtcNow.ToString('o'))+' FATAL '+$_.Exception.ToString()) -ErrorAction SilentlyContinue
    throw
} finally {
    if($child){$child.Dispose()}
    $guard.Dispose()
}
