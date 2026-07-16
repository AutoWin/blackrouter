$ErrorActionPreference = "Stop"

$base = "C:\BlackRouter"
$envFile = "$base\config\blackrouter.env"
$log = "$base\logs\blackrouter.log"

Get-Content -LiteralPath $envFile | ForEach-Object {
    if ($_ -match "^([^#=][^=]*)=(.*)$") {
        [Environment]::SetEnvironmentVariable($matches[1].Trim(), $matches[2], "Process")
    }
}

Set-Location $base
$ErrorActionPreference = "Continue"
& "$base\bin\blackrouter.exe" *>> $log
exit $LASTEXITCODE
