# Claude Desktop account switcher.
#
# Closes Claude Desktop, saves the signed-in account's login into
# <Profiles>\<accountUuid>, then either restores another saved login
# (-Mode switch) or clears the login so Claude shows its sign-in page
# (-Mode add), and starts Claude again.
#
# Only login data is swapped: the session cookies, the web storage of the
# claude.ai views and two keys of config.json. Everything else stays.
param(
  [Parameter(Mandatory = $true)][ValidateSet('switch', 'add')][string]$Mode,
  [string]$From = '',
  [string]$To = '',
  [Parameter(Mandatory = $true)][string]$UserData,
  [Parameter(Mandatory = $true)][string]$Profiles,
  # For tests: leave the Claude process alone.
  [switch]$NoAppControl
)

$ErrorActionPreference = 'Stop'
$Uuid = '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
$Items = @('Network\Cookies', 'Network\Cookies-journal', 'Local Storage', 'Session Storage', 'IndexedDB')
$AuthKeys = @('oauth:tokenCacheV2', 'lastKnownAccountUuid')
$ConfigPath = Join-Path $UserData 'config.json'
$LogPath = Join-Path (Split-Path $Profiles -Parent) 'switch.log'

function Write-Log([string]$Message) {
  New-Item -ItemType Directory -Force -Path (Split-Path $LogPath -Parent) | Out-Null
  Add-Content -LiteralPath $LogPath -Value ('{0:yyyy-MM-dd HH:mm:ss}  {1}' -f (Get-Date), $Message) -Encoding UTF8
}

Add-Type -AssemblyName System.Web.Extensions
function New-Serializer {
  $s = New-Object System.Web.Script.Serialization.JavaScriptSerializer
  $s.MaxJsonLength = [int]::MaxValue
  $s.RecursionLimit = 1000
  return $s
}
function Read-Json([string]$Path) {
  return (New-Serializer).DeserializeObject([IO.File]::ReadAllText($Path))
}
function Write-Json([string]$Path, $Value) {
  $tmp = "$Path.switcher-tmp"
  [IO.File]::WriteAllText($tmp, (New-Serializer).Serialize($Value), (New-Object Text.UTF8Encoding $false))
  Move-Item -LiteralPath $tmp -Destination $Path -Force
}

function Remove-PathIfAny([string]$Path) {
  if (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Recurse -Force }
}

# Replaces $Destination with a copy of $Source; a missing source leaves no destination.
function Copy-Replace([string]$Source, [string]$Destination) {
  Remove-PathIfAny $Destination
  if (Test-Path -LiteralPath $Source -PathType Container) {
    $null = robocopy $Source $Destination /E /R:2 /W:1 /NFL /NDL /NJH /NJS /NP
    if ($LASTEXITCODE -ge 8) { throw "robocopy '$Source' -> '$Destination' failed ($LASTEXITCODE)" }
  }
  elseif (Test-Path -LiteralPath $Source -PathType Leaf) {
    New-Item -ItemType Directory -Force -Path (Split-Path $Destination -Parent) | Out-Null
    Copy-Item -LiteralPath $Source -Destination $Destination -Force
  }
}

function Get-SignedInAccount {
  if (-not (Test-Path -LiteralPath $ConfigPath)) { return '' }
  $config = Read-Json $ConfigPath
  if ($config.ContainsKey('oauth:tokenCacheV2') -and $config.ContainsKey('lastKnownAccountUuid')) {
    return [string]$config['lastKnownAccountUuid']
  }
  return ''
}

function Save-Profile([string]$Id) {
  $dir = Join-Path $Profiles $Id
  $new = "$dir.new"
  $old = "$dir.old"
  Remove-PathIfAny $new
  New-Item -ItemType Directory -Force -Path $new | Out-Null
  foreach ($item in $Items) { Copy-Replace (Join-Path $UserData $item) (Join-Path $new "data\$item") }
  $config = Read-Json $ConfigPath
  $auth = @{}
  foreach ($key in $AuthKeys) { if ($config.ContainsKey($key)) { $auth[$key] = $config[$key] } }
  Write-Json (Join-Path $new 'auth.json') $auth
  # Swap in the new snapshot only once it is complete.
  Remove-PathIfAny $old
  if (Test-Path -LiteralPath $dir) { Rename-Item -LiteralPath $dir -NewName "$Id.old" }
  Rename-Item -LiteralPath $new -NewName $Id
  Remove-PathIfAny $old
}

function Set-AuthKeys($Auth) {
  $config = Read-Json $ConfigPath
  foreach ($key in $AuthKeys) {
    if ($Auth -and $Auth.ContainsKey($key)) { $config[$key] = $Auth[$key] }
    elseif ($config.ContainsKey($key)) { [void]$config.Remove($key) }
  }
  Write-Json $ConfigPath $config
}

function Restore-Profile([string]$Id) {
  $dir = Join-Path $Profiles $Id
  $authPath = Join-Path $dir 'auth.json'
  if (-not (Test-Path -LiteralPath $authPath)) { throw "no saved login for $Id" }
  foreach ($item in $Items) { Copy-Replace (Join-Path $dir "data\$item") (Join-Path $UserData $item) }
  Set-AuthKeys (Read-Json $authPath)
}

function Clear-Login {
  foreach ($item in $Items) { Remove-PathIfAny (Join-Path $UserData $item) }
  Set-AuthKeys $null
}

function Get-ClaudeProcesses {
  $engine = Join-Path $UserData 'claude-code'
  Get-Process -Name 'claude' -ErrorAction SilentlyContinue | Where-Object {
    $path = $_.Path
    $path -and ($path -like '*\WindowsApps\Claude_*' -or $path.StartsWith($engine, [StringComparison]::OrdinalIgnoreCase))
  }
}

function Wait-NoClaude([int]$Seconds) {
  $until = (Get-Date).AddSeconds($Seconds)
  while ((Get-Date) -lt $until) {
    if (-not (Get-ClaudeProcesses)) { return $true }
    Start-Sleep -Milliseconds 250
  }
  return -not (Get-ClaudeProcesses)
}

function Stop-Claude {
  foreach ($p in Get-ClaudeProcesses) {
    if ($p.MainWindowHandle -ne [IntPtr]::Zero) { [void]$p.CloseMainWindow() }
  }
  if (-not (Wait-NoClaude 8)) {
    Get-ClaudeProcesses | Stop-Process -Force -ErrorAction SilentlyContinue
    if (-not (Wait-NoClaude 10)) { throw 'Claude did not exit' }
  }
  Start-Sleep -Milliseconds 800
}

function Start-Claude {
  $family = (Get-AppxPackage -Name 'Claude' -ErrorAction SilentlyContinue | Select-Object -First 1).PackageFamilyName
  if (-not $family) {
    # The data folder is named after the package's publisher id: claude_<id>.
    $family = 'Claude_' + ((Split-Path $UserData -Leaf) -replace '^claude_', '')
  }
  Start-Process "shell:AppsFolder\$family!Claude"
}

$mutex = New-Object Threading.Mutex($false, 'Local\ClaudeAccountSwitcher')
if (-not $mutex.WaitOne(0)) { Write-Log 'another switch is running'; exit 2 }

$exitCode = 0
try {
  if ($From -and $From -notmatch $Uuid) { throw "bad -From '$From'" }
  if ($Mode -eq 'switch') {
    if ($To -notmatch $Uuid) { throw "bad -To '$To'" }
    if (-not (Test-Path -LiteralPath (Join-Path $Profiles "$To\auth.json"))) { throw "no saved login for $To" }
  }
  if (-not (Test-Path -LiteralPath $ConfigPath)) { throw "no config.json in $UserData" }

  if (-not $NoAppControl) { Stop-Claude }

  $current = Get-SignedInAccount
  if ($current) {
    if ($From -and $current -ne $From) { throw "signed in is $current, expected $From; nothing changed" }
    Save-Profile $current
    Write-Log "saved $current"
  }

  try {
    if ($Mode -eq 'switch') { Restore-Profile $To; Write-Log "restored $To" }
    else { Clear-Login; Write-Log 'cleared login for a new account' }
  }
  catch {
    Write-Log "restore failed: $($_.Exception.Message); putting $current back"
    if ($current) { Restore-Profile $current }
    throw
  }
}
catch {
  Write-Log "error: $($_.Exception.Message)"
  $exitCode = 1
}
finally {
  if (-not $NoAppControl) {
    try { Start-Claude } catch { Write-Log "start failed: $($_.Exception.Message)" }
  }
  $mutex.ReleaseMutex()
}
exit $exitCode
