# Builds claude-accounts.ico: Claude's app logo with a dark "switch" badge
# (two arrows) in the lower right, at every size Windows asks for.
# Run again after a Claude update changes the logo; install.ps1 calls it.
param([string]$Out = (Join-Path $PSScriptRoot 'claude-accounts.ico'))

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$package = Get-AppxPackage -Name 'Claude' | Select-Object -First 1
if (-not $package) { throw 'Claude Desktop is not installed' }
$assets = Join-Path $package.InstallLocation 'assets'

function Get-Logo([int]$Size) {
  # The smallest packaged logo at least this big, else the largest.
  $candidates = Get-ChildItem -LiteralPath $assets -Filter 'Square44x44Logo.targetsize-*.png' |
    Where-Object { $_.Name -match '^Square44x44Logo\.targetsize-(\d+)\.png$' } |
    ForEach-Object { [pscustomobject]@{ Size = [int]$Matches[1]; Path = $_.FullName } } |
    Sort-Object Size
  $pick = $candidates | Where-Object { $_.Size -ge $Size } | Select-Object -First 1
  if (-not $pick) { $pick = $candidates | Select-Object -Last 1 }
  return [System.Drawing.Image]::FromFile($pick.Path)
}

function New-Frame([int]$Size) {
  $bitmap = New-Object System.Drawing.Bitmap($Size, $Size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $g = [System.Drawing.Graphics]::FromImage($bitmap)
  $g.SmoothingMode = 'AntiAlias'
  $g.InterpolationMode = 'HighQualityBicubic'
  $g.PixelOffsetMode = 'HighQuality'

  $logo = Get-Logo $Size
  $g.DrawImage($logo, 0, 0, $Size, $Size)
  $logo.Dispose()

  # Badge: dark disc with a light ring, lower right.
  $d = [math]::Round($Size * 0.58)
  $x = $Size - $d
  $y = $Size - $d
  $ring = [math]::Max(1, [math]::Round($Size / 16))
  $g.FillEllipse((New-Object System.Drawing.SolidBrush([System.Drawing.Color]::White)), $x, $y, $d, $d)
  $g.FillEllipse((New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 32, 32, 36))), $x + $ring, $y + $ring, $d - 2 * $ring, $d - 2 * $ring)

  # Two opposite arrows: upper one points right, lower one points left.
  $pen = New-Object System.Drawing.Pen([System.Drawing.Color]::White, [single][math]::Max(1.2, $Size / 14))
  $pen.StartCap = 'Round'
  $pen.EndCap = 'Round'
  $cx = $x + $d / 2
  $cy = $y + $d / 2
  $w = $d * 0.26
  $gap = $d * 0.15
  $head = $d * 0.17
  $g.DrawLine($pen, [single]($cx - $w), [single]($cy - $gap), [single]($cx + $w), [single]($cy - $gap))
  $g.DrawLine($pen, [single]($cx + $w), [single]($cy - $gap), [single]($cx + $w - $head), [single]($cy - $gap - $head))
  $g.DrawLine($pen, [single]($cx + $w), [single]($cy + $gap), [single]($cx - $w), [single]($cy + $gap))
  $g.DrawLine($pen, [single]($cx - $w), [single]($cy + $gap), [single]($cx - $w + $head), [single]($cy + $gap + $head))

  $g.Dispose()
  $stream = New-Object System.IO.MemoryStream
  if ($Size -ge 256) {
    $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
  }
  else {
    # Classic 32-bit DIB frame: System.Drawing (and so the tray icon) misreads
    # PNG frames below 256 px.
    $rect = New-Object System.Drawing.Rectangle(0, 0, $Size, $Size)
    $data = $bitmap.LockBits($rect, 'ReadOnly', [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $pixels = New-Object byte[] ($data.Stride * $Size)
    [Runtime.InteropServices.Marshal]::Copy($data.Scan0, $pixels, 0, $pixels.Length)
    $bitmap.UnlockBits($data)
    $bw = New-Object System.IO.BinaryWriter($stream)
    $maskStride = [math]::Ceiling($Size / 32) * 4
    $bw.Write([uint32]40); $bw.Write([int32]$Size); $bw.Write([int32]($Size * 2))
    $bw.Write([uint16]1); $bw.Write([uint16]32); $bw.Write([uint32]0)
    $bw.Write([uint32]($Size * $Size * 4 + $maskStride * $Size))
    $bw.Write([int32]0); $bw.Write([int32]0); $bw.Write([uint32]0); $bw.Write([uint32]0)
    for ($row = $Size - 1; $row -ge 0; $row--) { $bw.Write($pixels, $row * $data.Stride, $Size * 4) }
    $bw.Write((New-Object byte[] ($maskStride * $Size)))
    $bw.Flush()
  }
  $bitmap.Dispose()
  return $stream
}

# ICO container: DIB frames, PNG for 256 px.
$sizes = 16, 20, 24, 32, 40, 48, 64, 256
$frames = New-Object 'System.Collections.Generic.List[byte[]]'
foreach ($s in $sizes) { $frames.Add((New-Frame $s).ToArray()) }
$ico = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter($ico)
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
  $s = $sizes[$i]
  $w.Write([byte]($(if ($s -ge 256) { 0 } else { $s }))); $w.Write([byte]($(if ($s -ge 256) { 0 } else { $s })))
  $w.Write([byte]0); $w.Write([byte]0); $w.Write([uint16]1); $w.Write([uint16]32)
  $w.Write([uint32]$frames[$i].Length); $w.Write([uint32]$offset)
  $offset += $frames[$i].Length
}
foreach ($f in $frames) { $w.Write($f) }
$w.Flush()
[IO.File]::WriteAllBytes($Out, $ico.ToArray())
"Icon: $Out"
