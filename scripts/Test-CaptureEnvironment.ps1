#requires -Version 5.1
[CmdletBinding()]
param([switch]$RequireRealtimeApi, [switch]$RequireCaptureApi, [ValidateSet("auto","realtime","etw")][string]$Backend = "auto")
$ErrorActionPreference = 'Stop'
# Capability check only: no driver/session is opened, no elevation or game is needed.
if (-not ('VisionOverlay.CaptureEnvironmentNative' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace VisionOverlay {
    public static class CaptureEnvironmentNative {
        [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        public static extern IntPtr LoadLibraryExW(string path, IntPtr file, uint flags);
        [DllImport("kernel32.dll", CharSet=CharSet.Ansi, ExactSpelling=true, SetLastError=true)]
        public static extern IntPtr GetProcAddress(IntPtr module, string name);
        [DllImport("kernel32.dll")]
        public static extern bool FreeLibrary(IntPtr module);
    }
}
'@
}
$version = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
$dll = Join-Path ([Environment]::SystemDirectory) 'PktMonApi.dll'
$required = @(
    'PacketMonitorInitialize','PacketMonitorUninitialize','PacketMonitorCreateLiveSession',
    'PacketMonitorSetSessionActive','PacketMonitorCreateRealtimeStream','PacketMonitorAttachOutputToSession',
    'PacketMonitorCloseSessionHandle','PacketMonitorCloseRealtimeStream','PacketMonitorAddCaptureConstraint',
    'PacketMonitorEnumDataSources','PacketMonitorAddSingleDataSourceToSession'
)
$legacyRequired = @('PktmonAddFilter','PktmonRemoveAllFilters','PktmonStart','PktmonStop','PktmonGetStatus','PktmonUnload')
$missingLegacy = @()
$missing = @()
$loadError = $null
$module = [VisionOverlay.CaptureEnvironmentNative]::LoadLibraryExW($dll, [IntPtr]::Zero, 0x800)
if ($module -eq [IntPtr]::Zero) {
    $loadError = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    $missing = $required
    $missingLegacy = $legacyRequired
} else {
    try {
        $missing = @($required | Where-Object {
            [VisionOverlay.CaptureEnvironmentNative]::GetProcAddress($module, $_) -eq [IntPtr]::Zero
        })
        $missingLegacy = @($legacyRequired | Where-Object {
            [VisionOverlay.CaptureEnvironmentNative]::GetProcAddress($module, $_) -eq [IntPtr]::Zero
        })
    } finally {
        [void][VisionOverlay.CaptureEnvironmentNative]::FreeLibrary($module)
    }
}
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
$report = [ordered]@{
    schema_version = 1
    windows_build = "$($version.CurrentBuildNumber).$($version.UBR)"
    windows_release = $version.DisplayVersion
    process_64_bit = [Environment]::Is64BitProcess
    elevated = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    pktmon_dll_exists = (Test-Path -LiteralPath $dll -PathType Leaf)
    pktmon_dll_version = $(if (Test-Path -LiteralPath $dll -PathType Leaf) { (Get-Item -LiteralPath $dll).VersionInfo.FileVersion } else { $null })
    dll_load_error = $loadError
    missing_realtime_functions = @($missing)
    missing_etw_control_functions = @($missingLegacy)
    etw_control_api_available = ($module -ne [IntPtr]::Zero -and $missingLegacy.Count -eq 0 -and [Environment]::Is64BitProcess)
    realtime_api_available = ($module -ne [IntPtr]::Zero -and $missing.Count -eq 0 -and [Environment]::Is64BitProcess)
}
$selected = if ($Backend -eq 'auto') { if ($report.realtime_api_available) {'realtime'} else {'etw'} } else {$Backend}
$report.selected_backend = $selected
$report.capture_api_available = if ($selected -eq 'realtime') {$report.realtime_api_available} else {$report.etw_control_api_available}
if (($RequireRealtimeApi -and !$report.realtime_api_available) -or ($RequireCaptureApi -and !$report.capture_api_available)) {
    throw "Capture API unavailable on Windows build $($report.windows_build). Requested backend: $Backend. Missing realtime exports: $($missing -join ', '). Missing ETW control exports: $($missingLegacy -join ', '). DLL load error: $loadError. Administrator rights cannot add missing API functions. Run Test-CaptureEnvironment.ps1 for the diagnostic report."
}
$report | ConvertTo-Json -Depth 3
