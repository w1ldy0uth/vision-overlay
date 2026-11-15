# Pure launch policy: one attempt per game process lifetime; enumeration failures do not mean exit.
function Write-AutoCaptureJsonFile {
    param([string]$Path, [string]$Json)
    $temporary=$Path+'.tmp'
    [IO.File]::WriteAllText($temporary,$Json,[Text.UTF8Encoding]::new($false))
    if (Test-Path -LiteralPath $Path) {
        # PS5.1 coerces ordinary $null to an empty string for this .NET argument.
        for($attempt=0;$attempt -lt 4;$attempt++) {
            try { [IO.File]::Replace($temporary,$Path,[System.Management.Automation.Language.NullString]::Value);break }
            catch { if($attempt -eq 3){throw};Start-Sleep -Milliseconds 50 }
        }
    } else { [IO.File]::Move($temporary,$Path) }
}
function Get-AutoCaptureDecision {
    param([hashtable]$State, [AllowNull()][string]$GameKey, [bool]$ObservationKnown, [bool]$CaptureRunning, [bool]$Disabled)
    if (!$ObservationKnown) { return 'observation_failed' }
    if ($Disabled) { return 'disabled' }
    if (!$State.Initialized) {
        $State.Initialized = $true
        if ($GameKey) { $State.Attempted = $GameKey; return 'already_running' }
    }
    if (!$GameKey) { $State.Attempted = $null; return 'waiting' }
    if ($State.Attempted -eq $GameKey) { return 'handled' }
    if ($CaptureRunning) {
        # A manual capture (or the previous game's draining capture) already owns acquisition.
        $State.Attempted = $GameKey
        return 'external_capture'
    }
    $State.Attempted = $GameKey
    return 'launch'
}

# The privileged helper accepts only this closed command vocabulary, never shell text or paths.
function Test-AutoCaptureCommand {
    param($Command,[string]$Instance,[long]$NowMs)
    $allowed=@('schema_version','request_id','watcher_instance','action','target_run_id','expires_unix_ms')
    if (!$Command -or @($Command.PSObject.Properties | Where-Object {$_.Name -notin $allowed}).Count) { return 'invalid_command' }
    if ($Command.schema_version -ne 1 -or $Command.action -notin @('start','stop')) { return 'invalid_command' }
    $id=[guid]::Empty
    if (![guid]::TryParse([string]$Command.request_id,[ref]$id) -or $id -eq [guid]::Empty) { return 'invalid_command' }
    if ($Command.watcher_instance -ne $Instance) { return 'helper_restarted' }
    $expiry=0L
    if (![long]::TryParse([string]$Command.expires_unix_ms,[ref]$expiry) -or $expiry -lt $NowMs -or $expiry -gt $NowMs+60000) { return 'expired_command' }
    if ($Command.action -eq 'stop' -and (![guid]::TryParse([string]$Command.target_run_id,[ref]$id) -or $id -eq [guid]::Empty)) { return 'invalid_target' }
    if ($Command.action -eq 'start' -and $Command.target_run_id) { return 'invalid_target' }
    return 'valid'
}
