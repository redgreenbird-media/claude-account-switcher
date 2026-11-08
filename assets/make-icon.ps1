# Draws assets/icon.png (1024 px): two people and a swap badge on an orange tile.
# `npx tauri icon assets/icon.png -o src-tauri/icons` builds every size from it.
Add-Type -AssemblyName System.Drawing
$s = 1024
$bmp = New-Object System.Drawing.Bitmap($s, $s, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = 'AntiAlias'
$g.Clear([System.Drawing.Color]::Transparent)

function RoundRect($x, $y, $w, $h, $r) {
  $p = New-Object System.Drawing.Drawing2D.GraphicsPath
  $p.AddArc($x, $y, 2 * $r, 2 * $r, 180, 90)
  $p.AddArc($x + $w - 2 * $r, $y, 2 * $r, 2 * $r, 270, 90)
  $p.AddArc($x + $w - 2 * $r, $y + $h - 2 * $r, 2 * $r, 2 * $r, 0, 90)
  $p.AddArc($x, $y + $h - 2 * $r, 2 * $r, 2 * $r, 90, 90)
  $p.CloseFigure()
  return $p
}
$orange = [System.Drawing.Color]::FromArgb(255, 217, 119, 87)
$tile = RoundRect 32 32 960 960 220
$g.FillPath((New-Object System.Drawing.SolidBrush($orange)), $tile)
$g.SetClip($tile)

function Person($cx, $top, $color) {
  $b = New-Object System.Drawing.SolidBrush($color)
  $g.FillEllipse($b, $cx - 115, $top, 230, 230)
  $g.FillEllipse($b, $cx - 200, $top + 260, 400, 420)
}
Person 380 190 ([System.Drawing.Color]::FromArgb(255, 251, 222, 206))
# Gap ring around the front person, so the two read apart.
$gap = New-Object System.Drawing.SolidBrush($orange)
$g.FillEllipse($gap, 590 - 145, 250 - 30, 290, 290)
$g.FillEllipse($gap, 590 - 230, 510 - 30, 460, 480)
Person 590 250 ([System.Drawing.Color]::White)

# Badge lower right: orange ring, white disc, orange swap arrows.
$g.ResetClip()
$g.FillEllipse($gap, 560, 560, 420, 420)
$g.FillEllipse((New-Object System.Drawing.SolidBrush([System.Drawing.Color]::White)), 600, 600, 340, 340)
$pen = New-Object System.Drawing.Pen($orange, 44)
$pen.StartCap = 'Round'; $pen.EndCap = 'Round'; $pen.LineJoin = 'Round'
$g.DrawLines($pen, [System.Drawing.Point[]]@((New-Object System.Drawing.Point 680, 720), (New-Object System.Drawing.Point 860, 720), (New-Object System.Drawing.Point 805, 670)))
$g.DrawLines($pen, [System.Drawing.Point[]]@((New-Object System.Drawing.Point 860, 820), (New-Object System.Drawing.Point 680, 820), (New-Object System.Drawing.Point 735, 870)))

$bmp.Save((Join-Path $PSScriptRoot 'icon.png'), [System.Drawing.Imaging.ImageFormat]::Png)
