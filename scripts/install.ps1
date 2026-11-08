# Installs the Claude Accounts tray icon:
# builds the icon, adds "Claude Accounts" to Autostart and the Start menu,
# and starts it. Safe to run again (after a Claude update, to refresh the icon).
$ErrorActionPreference = 'Stop'

& (Join-Path $PSScriptRoot 'make-icon.ps1') | Out-Null

$tray = Join-Path $PSScriptRoot 'tray.ps1'
$icon = Join-Path $PSScriptRoot 'claude-accounts.ico'
$shell = New-Object -ComObject WScript.Shell

foreach ($folder in [Environment]::GetFolderPath('Startup'), [Environment]::GetFolderPath('Programs')) {
  $link = $shell.CreateShortcut((Join-Path $folder 'Claude Accounts.lnk'))
  # conhost --headless keeps a console window from flashing up.
  $link.TargetPath = "$env:SystemRoot\System32\conhost.exe"
  $link.Arguments = "--headless powershell.exe -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$tray`""
  $link.WorkingDirectory = $PSScriptRoot
  $link.IconLocation = "$icon,0"
  $link.Description = 'Claude-Account wechseln'
  $link.Save()
}

# Starts the tray icon now (a running one just opens its panel).
Start-Process -FilePath "$env:SystemRoot\System32\conhost.exe" -ArgumentList "--headless powershell.exe -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$tray`"" -WorkingDirectory $PSScriptRoot
'Claude Accounts installed: tray icon, Autostart, Start menu.'
