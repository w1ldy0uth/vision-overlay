#requires -Version 5.1
[CmdletBinding()]
param([switch]$Disable,[switch]$DryRun)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'Runtime.Paths.ps1')
$sha=[Security.Cryptography.SHA256]::Create()
try { $suffix=([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($projectRoot.ToLowerInvariant())))).Replace('-','').Substring(0,16) } finally { $sha.Dispose() }
$taskName="VisionOverlay-AutoCapture-$suffix"
$script=Join-Path $PSScriptRoot 'Watch-GameCapture.ps1'
$stateDir=Join-Path $dataRoot 'auto-capture'
if ($DryRun) {
    Write-Output "Task: $taskName; action=$(if ($Disable) {'disable'} else {'enable'})."
    Write-Output "Watcher: $script. Current-user logon, interactive token, highest privileges, hidden, no password stored."
    Write-Output 'Enabling starts the watcher immediately. Disable prevents new starts and lets an active capture finish.'
    return
}
$identity=[Security.Principal.WindowsIdentity]::GetCurrent()
$principal=[Security.Principal.WindowsPrincipal]::new($identity)
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Run this configuration once in an administrator PowerShell terminal.' }
New-Item -ItemType Directory -Path $stateDir -Force | Out-Null
$disabledFile=Join-Path $stateDir 'disabled'
if ($Disable) {
    [IO.File]::WriteAllText($disabledFile,'disabled',[Text.UTF8Encoding]::new($false))
    $task=Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
    if ($task) { $task | Enable-ScheduledTask | Out-Null; Start-ScheduledTask -TaskName $taskName }
    Write-Output 'Automatic starts disabled. The helper stays available for manual web commands; an active capture can finish normally.'
    return
}
foreach ($required in $script,(Join-Path $binaryRoot 'capture-probe.exe'),(Join-Path $dataRoot 'gi.json')) {
    if (!(Test-Path -LiteralPath $required -PathType Leaf)) { throw "Required file missing: $required" }
}
if (Test-Path -LiteralPath $disabledFile) { Remove-Item -LiteralPath $disabledFile }
$action=New-ScheduledTaskAction -Execute (Join-Path $env:WINDIR 'System32/WindowsPowerShell/v1.0/powershell.exe') -Argument ('-NoProfile -ExecutionPolicy RemoteSigned -WindowStyle Hidden -File "'+$script+'"') -WorkingDirectory $projectRoot
$trigger=New-ScheduledTaskTrigger -AtLogOn -User $identity.Name
$runAs=New-ScheduledTaskPrincipal -UserId $identity.Name -LogonType Interactive -RunLevel Highest
$settings=New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1)
Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Principal $runAs -Settings $settings -Description 'Vision Overlay: watch game launches and start local inventory capture. No game memory access.' -Force | Out-Null
[IO.File]::WriteAllText((Join-Path $stateDir 'configured.json'),(@{schema_version=1;task_name=$taskName;configured_at=[DateTime]::UtcNow.ToString('o')} | ConvertTo-Json),[Text.UTF8Encoding]::new($false))
Start-ScheduledTask -TaskName $taskName
Write-Output 'Automatic capture enabled. Close an already-running game, then launch it normally. Check the web app for the snapshot.'
Write-Output "Logs and status: $stateDir"
