# Shared layout: repository defaults remain unchanged; portable packages carry a marker.
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ((Split-Path $projectRoot -Leaf) -eq 'components' -and (Test-Path -LiteralPath (Join-Path $projectRoot '../portable.json'))) {
    $projectRoot = [IO.Path]::GetFullPath((Join-Path $projectRoot '..'))
    $dataRoot = Join-Path $projectRoot 'data'
    $binaryRoot = Join-Path $projectRoot 'components'
} else {
    $dataRoot = Join-Path $projectRoot 'local-data'
    $binaryRoot = Join-Path $projectRoot 'target/debug'
}
