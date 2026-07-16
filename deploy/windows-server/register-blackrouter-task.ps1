$ErrorActionPreference = "Stop"

$action = New-ScheduledTaskAction `
    -Execute "powershell.exe" `
    -Argument '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "C:\BlackRouter\service\start-blackrouter.ps1"'
$trigger = New-ScheduledTaskTrigger -AtStartup
$principal = New-ScheduledTaskPrincipal `
    -UserId "SYSTEM" `
    -LogonType ServiceAccount `
    -RunLevel Highest
$settings = New-ScheduledTaskSettingsSet `
    -ExecutionTimeLimit (New-TimeSpan -Days 3650) `
    -RestartCount 5 `
    -RestartInterval (New-TimeSpan -Minutes 1) `
    -StartWhenAvailable

Register-ScheduledTask `
    -TaskName "BlackRouter" `
    -Action $action `
    -Trigger $trigger `
    -Principal $principal `
    -Settings $settings `
    -Description "BlackRouter OpenAI-compatible LLM gateway on loopback port 20129." `
    -Force | Out-Null

Get-ScheduledTask -TaskName "BlackRouter" | Select-Object TaskName, State
