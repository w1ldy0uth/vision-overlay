#requires -Version 5.1
[CmdletBinding()]
param([ValidateSet('all','legacy','missing')][string]$Scenario)
$ErrorActionPreference='Stop'
if (!$Scenario) {
    foreach ($case in 'all','legacy','missing') {
        & (Join-Path $env:WINDIR 'System32/WindowsPowerShell/v1.0/powershell.exe') -NoProfile -ExecutionPolicy RemoteSigned -File $PSCommandPath -Scenario $case
        if ($LASTEXITCODE -ne 0) { throw "Capture capability policy failed: $case" }
    }
    Write-Output 'Capture capability selection and rejection checks passed; no driver or session opened.'
    return
}
# Substitute only the read-only DLL probe in a fresh process. The production
# script still applies its real auto/forced backend decision and rejection logic.
Add-Type -TypeDefinition @'
using System;
namespace VisionOverlay {
 public static class CaptureEnvironmentNative {
  public static string Scenario;
  public static IntPtr LoadLibraryExW(string path, IntPtr file, uint flags) { return new IntPtr(1); }
  public static IntPtr GetProcAddress(IntPtr module, string name) {
   if (Scenario == "missing" || (Scenario == "legacy" && name.StartsWith("PacketMonitor"))) return IntPtr.Zero;
   return new IntPtr(1);
  }
  public static bool FreeLibrary(IntPtr module) { return true; }
 }
}
'@
[VisionOverlay.CaptureEnvironmentNative]::Scenario=$Scenario
$probe=Join-Path $PSScriptRoot 'Test-CaptureEnvironment.ps1'
$report=(& $probe | ConvertFrom-Json)
$expected=if ($Scenario -eq 'all') {'realtime'} else {'etw'}
if ($report.selected_backend -ne $expected) {throw 'Auto selection mismatch.'}
if ($report.capture_api_available -ne ($Scenario -ne 'missing')) {throw 'Availability mismatch.'}
if ($Scenario -eq 'missing') {
    $rejected=$false
    try {& $probe -RequireCaptureApi | Out-Null} catch {$rejected=$_.Exception.Message -match 'Capture API unavailable'}
    if (!$rejected) {throw 'Missing API must reject capture.'}
} else {
    & $probe -RequireCaptureApi | Out-Null
    $forced=(& $probe -Backend etw -RequireCaptureApi | ConvertFrom-Json)
    if ($forced.selected_backend -ne 'etw') {throw 'Forced ETW was ignored.'}
}
if ($Scenario -eq 'legacy') {
    $rejected=$false
    try {& $probe -RequireRealtimeApi | Out-Null} catch {$rejected=$_.Exception.Message -match 'Capture API unavailable'}
    if (!$rejected) {throw 'Forced realtime must reject missing realtime exports.'}
}
