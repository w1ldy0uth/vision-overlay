#requires -Version 5.1
$ErrorActionPreference = 'Stop'
$tokens = $null
$parseErrors = $null
$path = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../Start-EtlCapture.ps1'))
$ast = [Management.Automation.Language.Parser]::ParseFile($path,[ref]$tokens,[ref]$parseErrors)
if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
$predicate = $ast.Find({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Test-EmptyPktmonFilters'},$true)
if (!$predicate) { throw 'Missing preflight predicate' }
# Load only the pure predicate. Do not run capture or call pktmon.
Invoke-Expression $predicate.Extent.Text
$emptyCases = @("Packet Filters:`r`n    None`r`n", " Packet Filters:  None ", 'No filters.', 'No packet filters defined.', 'Нет фильтров')
foreach ($text in $emptyCases) {
    if (!(Test-EmptyPktmonFilters -Text $text)) { throw "Rejected empty list: $text" }
}
$nonemptyCases = @("Packet Filters:`r`n  1  Custom UDP 22101", "Packet Filters:`r`n    None`r`n  1 Custom UDP", '1 No filters UDP 22101', '', 'Access is denied.')
foreach ($text in $nonemptyCases) {
    if (Test-EmptyPktmonFilters -Text $text) { throw "Accepted nonempty or unknown list: $text" }
}
Write-Output 'ETL preflight: 10 cases passed; no capture or system changes.'
