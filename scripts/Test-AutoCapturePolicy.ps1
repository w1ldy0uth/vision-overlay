#requires -Version 5.1
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'AutoCapture.State.ps1')
function Expect([string]$Actual,[string]$Expected) { if ($Actual -ne $Expected) { throw "Expected $Expected, got $Actual" } }
$s=@{Initialized=$false;Attempted=$null}
Expect (Get-AutoCaptureDecision $s $null $true $false $false) 'waiting'
Expect (Get-AutoCaptureDecision $s '1:100' $true $false $false) 'launch'
Expect (Get-AutoCaptureDecision $s '1:100' $true $false $false) 'handled'
Expect (Get-AutoCaptureDecision $s $null $false $false $false) 'observation_failed'
Expect (Get-AutoCaptureDecision $s '1:100' $true $false $false) 'handled'
Expect (Get-AutoCaptureDecision $s $null $true $false $false) 'waiting'
Expect (Get-AutoCaptureDecision $s '1:200' $true $false $false) 'launch'
Expect (Get-AutoCaptureDecision $s $null $true $false $false) 'waiting'
Expect (Get-AutoCaptureDecision $s '2:300' $true $true $false) 'external_capture'
Expect (Get-AutoCaptureDecision $s '2:300' $true $false $false) 'handled'
Expect (Get-AutoCaptureDecision $s '3:400' $true $false $true) 'disabled'
Expect (Get-AutoCaptureDecision $s '3:400' $true $false $false) 'launch'
$s=@{Initialized=$false;Attempted=$null}
Expect (Get-AutoCaptureDecision $s '4:500' $true $false $false) 'already_running'
Expect (Get-AutoCaptureDecision $s '4:500' $true $false $false) 'handled'
Write-Output '14 auto-capture policy checks passed; no capture, game process or scheduled task was started.'
$fixture=Join-Path ([IO.Path]::GetTempPath()) ('vision-status-test-'+[guid]::NewGuid().ToString('N')+'.json')
try {
    for ($n=0;$n -lt 100;$n++) {
        $payload=@{schema_version=1;state='waiting';sequence=$n;capture_host_pid=$null} | ConvertTo-Json -Compress
        Write-AutoCaptureJsonFile -Path $fixture -Json $payload
        $actual=[IO.File]::ReadAllText($fixture) | ConvertFrom-Json
        if ($actual.sequence -ne $n -or $null -ne $actual.capture_host_pid) { throw 'Repeated atomic status write corrupted JSON.' }
        if (Test-Path -LiteralPath ($fixture+'.tmp')) { throw 'Successful status update left a temporary file.' }
    }
    $reader=[IO.File]::Open($fixture,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite)
    $blocked=$false
    try {Write-AutoCaptureJsonFile $fixture '{"sequence":100}'} catch {$blocked=$true} finally {$reader.Dispose()}
    if(!$blocked){throw 'Expected sharing conflict from a reader without delete sharing.'}
    if (([IO.File]::ReadAllText($fixture) | ConvertFrom-Json).sequence -ne 99) {throw 'Sharing conflict modified the previous status.'}
    Write-AutoCaptureJsonFile $fixture '{"sequence":100}'
    if (([IO.File]::ReadAllText($fixture) | ConvertFrom-Json).sequence -ne 100) {throw 'Status did not recover after reader closed.'}
    Write-Output '100 real atomic status writes passed (creation and repeated replacement).'
    Write-Output 'Status sharing conflict preserves the old file and recovers after reader close.'
} finally {
    foreach ($file in $fixture,($fixture+'.tmp')) { if (Test-Path -LiteralPath $file) { Remove-Item -LiteralPath $file } }
}
$instance=[guid]::NewGuid().ToString()
$now=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
$command=[pscustomobject]@{schema_version=1;request_id=[guid]::NewGuid().ToString();watcher_instance=$instance;action='start';target_run_id=$null;expires_unix_ms=[string]($now+30000)}
Expect (Test-AutoCaptureCommand $command $instance $now) 'valid'
$command.action='powershell'
Expect (Test-AutoCaptureCommand $command $instance $now) 'invalid_command'
$command.action='start';$command | Add-Member -NotePropertyName arguments -NotePropertyValue 'arbitrary'
Expect (Test-AutoCaptureCommand $command $instance $now) 'invalid_command'
$command.PSObject.Properties.Remove('arguments')
Expect (Test-AutoCaptureCommand $command ([guid]::NewGuid().ToString()) $now) 'helper_restarted'
$command.expires_unix_ms=[string]($now-1)
Expect (Test-AutoCaptureCommand $command $instance $now) 'expired_command'
$command.expires_unix_ms=[string]($now+60001)
Expect (Test-AutoCaptureCommand $command $instance $now) 'expired_command'
$command.expires_unix_ms=[string]($now+30000);$command.action='stop'
Expect (Test-AutoCaptureCommand $command $instance $now) 'invalid_target'
$command.target_run_id=[guid]::NewGuid().ToString()
Expect (Test-AutoCaptureCommand $command $instance $now) 'valid'
$command.action='start'
Expect (Test-AutoCaptureCommand $command $instance $now) 'invalid_target'
Write-Output '9 closed-protocol checks passed: action vocabulary, no extra arguments, helper identity, TTL and stop target.'
foreach ($expectedExit in 0,7) {
    $child=Start-Process -FilePath (Join-Path $env:WINDIR 'System32/WindowsPowerShell/v1.0/powershell.exe') -ArgumentList @('-NoProfile','-Command',('exit '+$expectedExit)) -WindowStyle Hidden -PassThru
    try {
        $null=$child.Handle
        $child.WaitForExit()
        if ($null -eq $child.ExitCode -or $child.ExitCode -ne $expectedExit) { throw 'Child process exit code was not retained.' }
    } finally {$child.Dispose()}
}
Write-Output 'Real hidden child process exit codes 0 and 7 retained in Windows PowerShell 5.1.'
