# Claude Accounts: a tray icon that switches Claude Desktop accounts.
#
# Runs once per Windows login (install.ps1 puts it in Autostart). A click on
# the icon opens a small panel above the tray: every saved account with its
# e-mail, name and last known 5h/weekly limits. A click on another account
# runs switch.ps1 (Claude closes, the login is swapped, Claude starts again).
#
# Names and e-mails are read from Claude's own cached account data; limits
# come from accounts.json, which the account-switcher mod writes from Code
# sessions. Starting the script a second time opens the panel of the first.
param(
  # For tests: print the accounts and exit.
  [switch]$DryRun,
  # For tests: save the panel as a PNG and exit.
  [string]$Snapshot = '',
  [ValidateSet('', 'light', 'dark')][string]$Theme = ''
)

$ErrorActionPreference = 'Stop'
$Root = Join-Path $env:LOCALAPPDATA 'ClaudeAccountSwitcher'
$Profiles = Join-Path $Root 'profiles'
$NamesPath = Join-Path $Root 'names.json'
$Switch = Join-Path $PSScriptRoot 'switch.ps1'
$IconPath = Join-Path $PSScriptRoot 'claude-accounts.ico'
$Uuid = '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'

# ---------- single instance ----------

$showEvent = New-Object Threading.EventWaitHandle($false, 'AutoReset', 'Local\ClaudeAccountsShow')
if (-not $DryRun -and -not $Snapshot) {
  $created = $false
  $script:instance = New-Object Threading.Mutex($true, 'Local\ClaudeAccountsTray', [ref]$created)
  if (-not $created) { [void]$showEvent.Set(); exit 0 }
}

# ---------- data ----------

function Read-JsonFile([string]$Path) {
  if (-not (Test-Path -LiteralPath $Path)) { return $null }
  try { return [IO.File]::ReadAllText($Path) | ConvertFrom-Json } catch { return $null }
}

function Read-Shared([string]$Path) {
  # Claude may hold the file open; read it without locking it.
  $stream = [IO.File]::Open($Path, 'Open', 'Read', 'ReadWrite, Delete')
  try {
    $bytes = New-Object byte[] $stream.Length
    [void]$stream.Read($bytes, 0, $bytes.Length)
    return $bytes
  }
  finally { $stream.Dispose() }
}

function Find-UserData {
  $known = Read-JsonFile (Join-Path $Root 'accounts.json')
  if ($known -and $known.userData -and (Test-Path -LiteralPath (Join-Path $known.userData 'config.json'))) { return [string]$known.userData }
  $found = Get-ChildItem -LiteralPath (Join-Path $env:LOCALAPPDATA 'Claude-Data') -Directory -Filter 'claude_*' -ErrorAction SilentlyContinue |
    Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'config.json') } |
    Sort-Object { (Get-Item -LiteralPath (Join-Path $_.FullName 'config.json')).LastWriteTime } -Descending |
    Select-Object -First 1
  if ($found) { return $found.FullName }
  return ''
}

function Get-CurrentId([string]$UserData) {
  if (-not $UserData) { return '' }
  $config = Read-JsonFile (Join-Path $UserData 'config.json')
  if ($config -and $config.'oauth:tokenCacheV2' -and [string]$config.lastKnownAccountUuid -match $Uuid) {
    return [string]$config.lastKnownAccountUuid
  }
  return ''
}

# Reads a V8-serialized one-byte string ('"', varint length, bytes) at $At.
function Read-V8String([byte[]]$Bytes, [int]$At) {
  if ($At -ge $Bytes.Length -or $Bytes[$At] -ne 0x22) { return $null }
  $length = 0; $shift = 0; $i = $At + 1
  while ($i -lt $Bytes.Length) {
    $b = $Bytes[$i]; $i++
    $length = $length -bor (($b -band 0x7f) -shl $shift)
    if (($b -band 0x80) -eq 0) { break }
    $shift += 7
  }
  if ($length -le 0 -or $length -gt 200 -or $i + $length -gt $Bytes.Length) { return $null }
  return [Text.Encoding]::UTF8.GetString($Bytes, $i, $length)
}

# Claude's web view caches the signed-in account (uuid, e-mail, name) in its
# IndexedDB blobs; reads the entry for each wanted id.
function Find-AccountNames([string]$DataDir, [string[]]$Ids) {
  $result = @{}
  $blobs = Join-Path $DataDir 'IndexedDB\https_claude.ai_0.indexeddb.blob'
  if (-not (Test-Path -LiteralPath $blobs)) { return $result }
  $latin = [Text.Encoding]::GetEncoding(28591)
  foreach ($file in Get-ChildItem -LiteralPath $blobs -Recurse -File -ErrorAction SilentlyContinue | Where-Object Length -lt 8MB) {
    try { $bytes = Read-Shared $file.FullName } catch { continue }
    $text = $latin.GetString($bytes)
    foreach ($id in $Ids) {
      if ($result.ContainsKey($id)) { continue }
      $at = $text.IndexOf($id)
      if ($at -lt 0) { continue }
      $mail = $text.IndexOf('email_address', $at)
      if ($mail -lt 0 -or $mail - $at -gt 200) { continue }
      $email = Read-V8String $bytes ($mail + 13)
      $nameAt = $text.IndexOf('full_name', $mail)
      $name = if ($nameAt -gt 0 -and $nameAt - $mail -lt 300) { Read-V8String $bytes ($nameAt + 9) } else { $null }
      if ($email -and $email -match '@') { $result[$id] = @{ email = $email; name = $name } }
    }
    if ($result.Count -eq $Ids.Count) { break }
  }
  return $result
}

function Get-Names([string]$UserData, [string]$CurrentId, [string[]]$Ids) {
  $names = @{}
  $cached = Read-JsonFile $NamesPath
  if ($cached) { foreach ($p in $cached.PSObject.Properties) { $names[$p.Name] = @{ email = $p.Value.email; name = $p.Value.name } } }
  $missing = @($Ids | Where-Object { -not $names.ContainsKey($_) })
  if ($missing.Count -eq 0) { return $names }
  foreach ($id in $missing) {
    $dir = if ($id -eq $CurrentId) { $UserData } else { Join-Path $Profiles "$id\data" }
    $found = Find-AccountNames $dir @($id)
    if ($found.ContainsKey($id)) { $names[$id] = $found[$id] }
  }
  try { [IO.File]::WriteAllText($NamesPath, ($names | ConvertTo-Json -Depth 4)) } catch {}
  return $names
}

function Format-Span([double]$Ms) {
  $minutes = [math]::Max(1, [math]::Round($Ms / 60000))
  if ($minutes -lt 60) { return "$minutes min" }
  $hours = [math]::Floor($minutes / 60)
  if ($hours -lt 24) { return "$hours h $($minutes % 60) min" }
  return "$([math]::Floor($hours / 24)) T $($hours % 24) h"
}

function Format-Clock([DateTimeOffset]$At) {
  $local = $At.LocalDateTime
  $today = (Get-Date).Date
  if ($local.Date -eq $today) { return 'heute ' + $local.ToString('HH:mm') }
  if ($local.Date -eq $today.AddDays(1)) { return 'morgen ' + $local.ToString('HH:mm') }
  return $local.ToString('ddd d.M. HH:mm', [Globalization.CultureInfo]'de-CH')
}

function Get-Model {
  $userData = Find-UserData
  $currentId = Get-CurrentId $userData
  $known = Read-JsonFile (Join-Path $Root 'accounts.json')

  $ids = New-Object System.Collections.Generic.List[string]
  if ($currentId) { $ids.Add($currentId) }
  Get-ChildItem -LiteralPath $Profiles -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match $Uuid -and (Test-Path -LiteralPath (Join-Path $_.FullName 'auth.json')) } |
    Sort-Object Name |
    ForEach-Object { if (-not $ids.Contains($_.Name)) { $ids.Add($_.Name) } }

  $names = Get-Names $userData $currentId $ids.ToArray()
  $now = [DateTimeOffset]::Now
  $accounts = foreach ($id in $ids) {
    $info = if ($known -and $known.accounts -and $known.accounts.PSObject.Properties[$id]) { $known.accounts.$id } else { $null }
    $email = if ($names.ContainsKey($id) -and $names[$id].email) { $names[$id].email } elseif ($info -and $info.email) { [string]$info.email } else { '' }
    $name = if ($names.ContainsKey($id) -and $names[$id].name) { [string]$names[$id].name } else { '' }

    $limits = foreach ($kind in 'five_hour', 'seven_day') {
      $limit = if ($info -and $info.limits) { $info.limits | Where-Object { $_.kind -eq $kind } | Select-Object -First 1 } else { $null }
      $label = @{ five_hour = '5 Std.'; seven_day = 'Woche' }[$kind]
      if (-not $limit) { [pscustomobject]@{ Label = $label; Percent = -1; Reset = '' }; continue }
      $percent = [math]::Round([double]$limit.percentUsed)
      $reset = ''
      $at = [DateTimeOffset]::MinValue
      if ($limit.resetsAt -and [DateTimeOffset]::TryParse([string]$limit.resetsAt, [ref]$at)) {
        if ($at -le $now) { $percent = 0; $reset = 'zurückgesetzt' }
        else { $reset = "Reset in $(Format-Span ($at - $now).TotalMilliseconds) · $(Format-Clock $at)" }
      }
      [pscustomobject]@{ Label = $label; Percent = $percent; Reset = $reset }
    }
    $age = ''
    if ($info -and $info.measuredAt) {
      $ms = $now.ToUnixTimeMilliseconds() - [double]$info.measuredAt
      $age = if ($ms -lt 120000) { 'gerade eben' } else { "vor $(Format-Span $ms)" }
    }
    [pscustomobject]@{
      Id         = $id
      Email      = $(if ($email) { $email } else { "Account $($id.Substring(0, 8))" })
      Name       = $name
      IsCurrent  = ($id -eq $currentId)
      HasProfile = (Test-Path -LiteralPath (Join-Path $Profiles "$id\auth.json"))
      Limits     = @($limits)
      Age        = $age
    }
  }
  return [pscustomobject]@{ UserData = $userData; CurrentId = $currentId; Accounts = @($accounts) }
}

function Start-Switch([string]$Mode, [string]$To, [string]$UserData) {
  $line = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-WindowStyle', 'Hidden', '-File', "`"$Switch`"",
    '-Mode', $Mode, '-UserData', "`"$UserData`"", '-Profiles', "`"$Profiles`"")
  if ($To) { $line += @('-To', $To) }
  Start-Process -FilePath 'powershell.exe' -ArgumentList $line -WindowStyle Hidden
}

if ($DryRun) {
  $m = Get-Model
  "userData: $($m.UserData)"
  foreach ($a in $m.Accounts) {
    $l = ($a.Limits | ForEach-Object { "$($_.Label) $(if ($_.Percent -ge 0) { "$($_.Percent)%" } else { '-' }) $($_.Reset)" }) -join ' | '
    '{0} {1} ({2})  {3}  {4}' -f $(if ($a.IsCurrent) { '*' } else { ' ' }), $a.Email, $a.Name, $l, $a.Age
  }
  exit 0
}

# ---------- panel ----------

Add-Type -AssemblyName PresentationFramework, PresentationCore, WindowsBase, System.Windows.Forms, System.Drawing

function Get-IsLight {
  if ($Theme) { return $Theme -eq 'light' }
  try { return (Get-ItemPropertyValue 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize' 'SystemUsesLightTheme') -eq 1 } catch { return $false }
}

function Esc([string]$Text) { return [Security.SecurityElement]::Escape($Text) }

$AvatarColors = '#D97757', '#5B8DEF', '#3FA66B', '#A66BD9', '#D9A23F', '#3FA6A6'

function Get-Initials([string]$Name, [string]$Email) {
  $source = if ($Name) { $Name } else { ($Email -split '@')[0] -replace '[._-]', ' ' }
  $parts = @($source -split '\s+' | Where-Object { $_ })
  if ($parts.Count -ge 2) { return ($parts[0].Substring(0, 1) + $parts[-1].Substring(0, 1)).ToUpper() }
  if ($parts.Count -eq 1) { return $parts[0].Substring(0, [math]::Min(2, $parts[0].Length)).ToUpper() }
  return '?'
}

function New-PanelXaml($Model, [bool]$IsLight) {
  $c = if ($IsLight) {
    @{ Bg = '#FBFBFB'; Card = '#FFFFFF'; Hover = '#F0F0F0'; Line = '#E5E5E5'; Text = '#1B1B1B'; Sub = '#616161'; Track = '#E6E6E6'; Active = '#FBEDE7' }
  } else {
    @{ Bg = '#232323'; Card = '#2B2B2B'; Hover = '#353535'; Line = '#3A3A3A'; Text = '#F3F3F3'; Sub = '#A8A8A8'; Track = '#3D3D3D'; Active = '#3A2C27' }
  }
  $accent = '#D97757'

  $rows = foreach ($a in $Model.Accounts) {
    $hash = 0; foreach ($ch in $a.Email.ToCharArray()) { $hash = ($hash * 31 + [int]$ch) % 9973 }
    $avatar = $AvatarColors[$hash % $AvatarColors.Count]
    $initials = Esc (Get-Initials $a.Name $a.Email)
    $title = Esc $(if ($a.Name) { $a.Name } else { ($a.Email -split '@')[0] })
    $badge = if ($a.IsCurrent) {
      "<Border Grid.Column='2' Background='$accent' CornerRadius='9' Padding='8,2' VerticalAlignment='Top'><TextBlock Text='Aktiv' Foreground='White' FontSize='11' FontWeight='SemiBold'/></Border>"
    } elseif ($a.HasProfile) {
      "<TextBlock Grid.Column='2' Text='Wechseln &#x203A;' Foreground='$accent' FontSize='12' FontWeight='SemiBold' VerticalAlignment='Top' Margin='0,2,0,0'/>"
    } else { '' }

    $bars = foreach ($l in $a.Limits) {
      $hasData = $l.Percent -ge 0
      $pct = if ($hasData) { [math]::Min(100, [math]::Max(0, $l.Percent)) } else { 0 }
      $fill = if ($pct -ge 90) { '#E5484D' } elseif ($pct -ge 70) { '#E59A3F' } else { $accent }
      $value = if ($hasData) { "$pct %" } else { '–' }
      $reset = Esc $l.Reset
      @"
<Grid Margin='0,6,0,0'>
  <Grid.ColumnDefinitions><ColumnDefinition Width='46'/><ColumnDefinition Width='*'/><ColumnDefinition Width='44'/></Grid.ColumnDefinitions>
  <TextBlock Text='$(Esc $l.Label)' Foreground='$($c.Sub)' FontSize='11.5' VerticalAlignment='Center'/>
  <Grid Grid.Column='1' Height='6' VerticalAlignment='Center' Margin='0,0,8,0'>
    <Border Background='$($c.Track)' CornerRadius='3'/>
    <Grid>
      <Grid.ColumnDefinitions><ColumnDefinition Width='$pct*'/><ColumnDefinition Width='$(100 - $pct)*'/></Grid.ColumnDefinitions>
      <Border Background='$fill' CornerRadius='3'/>
    </Grid>
  </Grid>
  <TextBlock Grid.Column='2' Text='$value' Foreground='$($c.Text)' FontSize='11.5' FontWeight='SemiBold' HorizontalAlignment='Right' VerticalAlignment='Center'/>
</Grid>
$(if ($reset) { "<TextBlock Text='$reset' Foreground='$($c.Sub)' FontSize='10.5' Margin='46,1,0,0'/>" })
"@
    }
    $foot = if ($a.Age) { "<TextBlock Text='Stand: $(Esc $a.Age)' Foreground='$($c.Sub)' FontSize='10.5' Margin='0,6,0,0' Opacity='0.8'/>" }
            elseif (-not ($a.Limits | Where-Object { $_.Percent -ge 0 })) { "<TextBlock Text='Limits erscheinen nach der nächsten Antwort in einer Code-Session dieses Accounts.' TextWrapping='Wrap' Foreground='$($c.Sub)' FontSize='10.5' Margin='0,6,0,0' Opacity='0.8'/>" }
            else { '' }
    $look = if (-not $a.IsCurrent -and $a.HasProfile) { "Style='{StaticResource Switchable}' Cursor='Hand'" }
            elseif ($a.IsCurrent) { "Background='$($c.Active)' BorderBrush='$accent'" }
            else { "Background='$($c.Card)' BorderBrush='$($c.Line)'" }
    @"
<Border Tag='$($a.Id)' BorderThickness='1' CornerRadius='10' Padding='12,10' Margin='0,0,0,8' $look>
  <StackPanel>
    <Grid>
      <Grid.ColumnDefinitions><ColumnDefinition Width='Auto'/><ColumnDefinition Width='*'/><ColumnDefinition Width='Auto'/></Grid.ColumnDefinitions>
      <Grid Width='36' Height='36' Margin='0,0,10,0' VerticalAlignment='Top'>
        <Ellipse Fill='$avatar'/>
        <TextBlock Text='$initials' Foreground='White' FontSize='13' FontWeight='SemiBold' HorizontalAlignment='Center' VerticalAlignment='Center'/>
      </Grid>
      <StackPanel Grid.Column='1' VerticalAlignment='Center'>
        <TextBlock Text='$title' Foreground='$($c.Text)' FontSize='13.5' FontWeight='SemiBold' TextTrimming='CharacterEllipsis'/>
        <TextBlock Text='$(Esc $a.Email)' Foreground='$($c.Sub)' FontSize='12' TextTrimming='CharacterEllipsis'/>
      </StackPanel>
      $badge
    </Grid>
    <StackPanel Margin='0,6,0,0'>$($bars -join "`n")</StackPanel>
    $foot
  </StackPanel>
</Border>
"@
  }
  if (-not $Model.UserData) { $rows = "<TextBlock Text='Claude Desktop nicht gefunden.' Foreground='#E5484D' Margin='0,0,0,8'/>" }

  return @"
<Window xmlns='http://schemas.microsoft.com/winfx/2006/xaml/presentation'
        xmlns:x='http://schemas.microsoft.com/winfx/2006/xaml'
        Title='Claude Accounts' Width='372' SizeToContent='Height' WindowStyle='None' AllowsTransparency='True'
        Background='Transparent' ResizeMode='NoResize' ShowInTaskbar='False' Topmost='True'
        FontFamily='Segoe UI Variable Text, Segoe UI' UseLayoutRounding='True'>
  <Window.Resources>
    <Style x:Key='Switchable' TargetType='Border'>
      <Setter Property='Background' Value='$($c.Card)'/>
      <Setter Property='BorderBrush' Value='$($c.Line)'/>
      <Style.Triggers>
        <Trigger Property='IsMouseOver' Value='True'>
          <Setter Property='Background' Value='$($c.Hover)'/>
          <Setter Property='BorderBrush' Value='$accent'/>
        </Trigger>
      </Style.Triggers>
    </Style>
    <Style x:Key='Flat' TargetType='Button'>
      <Setter Property='Foreground' Value='$($c.Text)'/>
      <Setter Property='Cursor' Value='Hand'/>
      <Setter Property='Template'>
        <Setter.Value>
          <ControlTemplate TargetType='Button'>
            <Border x:Name='b' Background='$($c.Card)' BorderBrush='$($c.Line)' BorderThickness='1' CornerRadius='8' Padding='12,8'>
              <ContentPresenter HorizontalAlignment='Center'/>
            </Border>
            <ControlTemplate.Triggers>
              <Trigger Property='IsMouseOver' Value='True'><Setter TargetName='b' Property='Background' Value='$($c.Hover)'/></Trigger>
            </ControlTemplate.Triggers>
          </ControlTemplate>
        </Setter.Value>
      </Setter>
    </Style>
  </Window.Resources>
  <Border Margin='14' Background='$($c.Bg)' BorderBrush='$($c.Line)' BorderThickness='1' CornerRadius='12' Padding='14'>
    <Border.Effect><DropShadowEffect BlurRadius='18' ShadowDepth='3' Opacity='0.28'/></Border.Effect>
    <StackPanel>
      <Grid Margin='2,0,2,12'>
        <StackPanel Orientation='Horizontal'>
          <Image x:Name='Logo' Width='20' Height='20' Margin='0,0,8,0' VerticalAlignment='Center'/>
          <TextBlock Text='Claude Accounts' Foreground='$($c.Text)' FontSize='14' FontWeight='SemiBold' VerticalAlignment='Center'/>
        </StackPanel>
        <TextBlock x:Name='Close' Text='&#xE711;' FontFamily='Segoe Fluent Icons, Segoe MDL2 Assets' FontSize='11' Foreground='$($c.Sub)' HorizontalAlignment='Right' VerticalAlignment='Center' Cursor='Hand' Padding='6'/>
      </Grid>
      <StackPanel x:Name='List'>$($rows -join "`n")</StackPanel>
      <TextBlock x:Name='Status' Foreground='$accent' FontSize='12' FontWeight='SemiBold' TextWrapping='Wrap' Margin='2,0,2,8' Visibility='Collapsed'/>
      <Button x:Name='Add' Style='{StaticResource Flat}'>
        <StackPanel Orientation='Horizontal'>
          <TextBlock Text='&#xE710;' FontFamily='Segoe Fluent Icons, Segoe MDL2 Assets' FontSize='12' VerticalAlignment='Center' Margin='0,0,8,0'/>
          <TextBlock Text='Account hinzufügen' FontSize='12.5' VerticalAlignment='Center'/>
        </StackPanel>
      </Button>
      <TextBlock Text='Beim Wechsel schliesst Claude kurz und startet mit dem anderen Login neu.' Foreground='$($c.Sub)' FontSize='10.5' TextWrapping='Wrap' Margin='2,8,2,0' Opacity='0.8'/>
    </StackPanel>
  </Border>
</Window>
"@
}

$script:panel = $null
$script:model = $null
$script:hiddenAt = [DateTime]::MinValue

function New-Panel {
  $script:model = Get-Model
  $xaml = New-PanelXaml $script:model (Get-IsLight)
  $window = [Windows.Markup.XamlReader]::Parse($xaml)

  $logo = $window.FindName('Logo')
  if (Test-Path -LiteralPath $IconPath) {
    $decoder = New-Object Windows.Media.Imaging.IconBitmapDecoder((New-Object Uri $IconPath), 'None', 'OnLoad')
    $logo.Source = $decoder.Frames | Sort-Object { [math]::Abs($_.PixelWidth - 40) } | Select-Object -First 1
  }
  $window.FindName('Close').Add_MouseLeftButtonUp({ $script:panel.Hide() })
  $window.FindName('Add').Add_Click({
      Show-Status 'Melde ab für einen neuen Account – Claude startet neu …'
      Start-Switch 'add' '' $script:model.UserData
    })
  foreach ($row in $window.FindName('List').Children) {
    if ($row.Style) {
      $row.Add_MouseLeftButtonUp({
          param($sender)
          $target = $script:model.Accounts | Where-Object { $_.Id -eq [string]$sender.Tag } | Select-Object -First 1
          Show-Status "Wechsle zu $($target.Email) – Claude startet neu …"
          Start-Switch 'switch' $target.Id $script:model.UserData
        })
    }
  }
  $window.Add_Deactivated({ if ($script:panel -and $script:panel.IsVisible) { $script:panel.Hide(); $script:hiddenAt = [DateTime]::Now } })
  $window.Add_KeyDown({ param($s, $e) if ($e.Key -eq 'Escape') { $script:panel.Hide() } })
  return $window
}

function Show-Status([string]$Text) {
  $status = $script:panel.FindName('Status')
  $status.Text = $Text
  $status.Visibility = 'Visible'
  $script:panel.FindName('List').IsEnabled = $false
  $script:panel.FindName('Add').IsEnabled = $false
  $timer = New-Object Windows.Threading.DispatcherTimer
  $timer.Interval = [TimeSpan]::FromSeconds(2.5)
  $timer.Add_Tick({ param($t) $t.Stop(); $script:panel.Hide() })
  $timer.Start()
}

function Show-Panel {
  if ($script:panel) { $script:panel.Close() }
  $script:panel = New-Panel
  $script:panel.Show()
  $script:panel.UpdateLayout()
  # Bottom right, above the taskbar, like Windows' own flyouts.
  $area = [Windows.SystemParameters]::WorkArea
  $script:panel.Left = $area.Right - $script:panel.ActualWidth
  $script:panel.Top = $area.Bottom - $script:panel.ActualHeight
  $script:panel.Activate()
  Update-Tooltip
}

function Update-Tooltip {
  $current = $script:model.Accounts | Where-Object IsCurrent | Select-Object -First 1
  $text = if ($current) { "Claude: $($current.Email)" } else { 'Claude Accounts' }
  $script:tray.Text = $text.Substring(0, [math]::Min(63, $text.Length))
}

if ($Snapshot) {
  $script:panel = New-Panel
  $script:panel.Left = -4000; $script:panel.Top = -4000
  $script:panel.Show(); $script:panel.UpdateLayout()
  $bitmap = New-Object Windows.Media.Imaging.RenderTargetBitmap([int]$script:panel.ActualWidth, [int]$script:panel.ActualHeight, 96, 96, [Windows.Media.PixelFormats]::Pbgra32)
  $bitmap.Render($script:panel)
  $encoder = New-Object Windows.Media.Imaging.PngBitmapEncoder
  $encoder.Frames.Add([Windows.Media.Imaging.BitmapFrame]::Create($bitmap))
  $file = [IO.File]::Create($Snapshot); $encoder.Save($file); $file.Dispose()
  exit 0
}

# ---------- tray ----------

$app = New-Object Windows.Application
$app.ShutdownMode = 'OnExplicitShutdown'

$script:tray = New-Object System.Windows.Forms.NotifyIcon
$script:tray.Icon = if (Test-Path -LiteralPath $IconPath) { New-Object System.Drawing.Icon($IconPath, [System.Windows.Forms.SystemInformation]::SmallIconSize) } else { [System.Drawing.SystemIcons]::Application }
$script:tray.Text = 'Claude Accounts'
$script:tray.Visible = $true

$menu = New-Object System.Windows.Forms.ContextMenuStrip
[void]$menu.Items.Add('Accounts öffnen', $null, { Show-Panel })
[void]$menu.Items.Add('Beenden', $null, { $script:tray.Visible = $false; $script:tray.Dispose(); $app.Shutdown() })
$script:tray.ContextMenuStrip = $menu
$script:tray.Add_MouseClick({
    param($s, $e)
    if ($e.Button -ne 'Left') { return }
    # The click on the icon itself takes focus from an open panel and hides it first.
    if ($script:panel -and $script:panel.IsVisible) { $script:panel.Hide() }
    elseif (([DateTime]::Now - $script:hiddenAt).TotalMilliseconds -gt 400) { Show-Panel }
  })

# A second start (Start menu entry) asks this one to open the panel.
$poll = New-Object Windows.Threading.DispatcherTimer
$poll.Interval = [TimeSpan]::FromMilliseconds(250)
$poll.Add_Tick({ if ($showEvent.WaitOne(0)) { Show-Panel } })
$poll.Start()

try { $script:model = Get-Model; Update-Tooltip } catch {}
[void]$app.Run()
