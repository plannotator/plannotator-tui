$ErrorActionPreference = "Stop"

$pluginRoot = $PSScriptRoot
$repositoryRoot = Split-Path -Parent $pluginRoot
$source = Join-Path $repositoryRoot "target/release/plannotator-tui.exe"
$destinationDirectory = Join-Path $pluginRoot "bin"
$destination = Join-Path $destinationDirectory "plannotator-tui.exe"

New-Item -ItemType Directory -Force $destinationDirectory | Out-Null
Copy-Item -LiteralPath $source -Destination $destination -Force
