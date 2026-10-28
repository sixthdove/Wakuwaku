#!/usr/bin/env pwsh
# The README's pictures (README.md, README.en.md), into docs/readme/:
#   card-<id>-<lang>-<mode>.jpg  her pages, as the gallery shoots them
#                                (scenes.js, section readme: README_CARDS)
#   hero-<lang>-<mode>.jpg       the picture at the top (readme.html): her
#                                wallpaper, her name, her capsule lit
#   btn-<id>-<lang>-<mode>.png   the buttons under it, on nothing
# each in Chinese and English (lang zh, en), Windows' dark and light (mode).
#
#   pwsh tools/preview/readme.ps1 [-Out <folder>]
#
# The cards by render.ps1; then readme.html from a folder of its own, the
# wallpaper and the capsule's shots beside it (an image from another folder
# may not be drawn by the time Edge shoots). Needs node and Edge; draws
# nothing on the screen.
param(
    [string]$Out = (Join-Path $PSScriptRoot '..\..\docs\readme'),
    [string]$Edge = 'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe'
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$here = $PSScriptRoot
$work = Join-Path ([IO.Path]::GetTempPath()) 'wakuwaku-readme'
New-Item -ItemType Directory -Force -Path $work, $Out | Out-Null
$Out = (Resolve-Path -LiteralPath $Out).Path
$langs = 'zh', 'en'
$modes = 'dark', 'light'
# The buttons in each language, as readme.html has them, left to right.
$buttons = @{ zh = 'download', 'gitee', 'guide', 'lang'; en = 'download', 'guide', 'lang' }

$jpeg = [System.Drawing.Imaging.ImageCodecInfo]::GetImageEncoders() | Where-Object MimeType -EQ 'image/jpeg'
function Save-Jpeg([string]$png, [string]$jpg, [long]$quality) {
    $img = [System.Drawing.Image]::FromFile($png)
    try {
        $p = New-Object System.Drawing.Imaging.EncoderParameters 1
        $p.Param[0] = New-Object System.Drawing.Imaging.EncoderParameter ([System.Drawing.Imaging.Encoder]::Quality), $quality
        $img.Save($jpg, $jpeg, $p)
    } finally { $img.Dispose() }
}
function Invoke-Shot([string]$url, [int]$w, [int]$h, [double]$scale, [string]$png, [switch]$Clear) {
    if (Test-Path -LiteralPath $png) { Remove-Item -LiteralPath $png -Force }
    $a = @(
        '--headless=new', '--hide-scrollbars', '--allow-file-access-from-files', '--no-first-run', '--no-default-browser-check',
        "--user-data-dir=$(Join-Path $work '.edge')", "--window-size=$w,$h", "--force-device-scale-factor=$scale",
        '--virtual-time-budget=4000', '--run-all-compositor-stages-before-draw', "--screenshot=$png", "`"$url`""
    )
    if ($Clear) { $a = @('--default-background-color=00000000') + $a }
    Start-Process -FilePath $Edge -ArgumentList $a -Wait -WindowStyle Hidden
    if (-not (Test-Path -LiteralPath $png)) { throw "Edge shot nothing for $url" }
}

# --- The cards ----------------------------------------------------------------------------
& pwsh -NoProfile -File (Join-Path $here 'render.ps1') -Out $work -Only 'readme-*' -NoOverview
if ($LASTEXITCODE -ne 0) { throw 'render.ps1 did not shoot every card' }
foreach ($png in Get-ChildItem -LiteralPath $work -Filter 'readme-*.png') {
    Save-Jpeg $png.FullName (Join-Path $Out ($png.Name -replace '^readme-', 'card-' -replace '\.png$', '.jpg')) 90
}

# --- The hero -----------------------------------------------------------------------------
# Her wallpaper, smaller (as wide as the hero draws it, at 1.5).
$src = [System.Drawing.Image]::FromFile((Resolve-Path (Join-Path $here '..\..\wallpapers\claude-cheek-rest.png')).Path)
try {
    $w = 2400; $h = [int]($src.Height * $w / $src.Width)
    $small = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($small)
    $g.InterpolationMode = 'HighQualityBicubic'
    $g.DrawImage($src, 0, 0, $w, $h)
    $g.Dispose()
    $small.Save((Join-Path $work 'wall.jpg'), [System.Drawing.Imaging.ImageFormat]::Jpeg)
    $small.Dispose()
} finally { $src.Dispose() }
Copy-Item -LiteralPath (Join-Path $here 'readme.html') -Destination $work -Force
$page = [Uri]::new((Join-Path $work 'readme.html')).AbsoluteUri
foreach ($l in $langs) {
    foreach ($m in $modes) {
        $png = Join-Path $work "hero-$l-$m.png"
        Invoke-Shot "${page}?part=hero&lang=$l&mode=$m" 1280 600 1.5 $png
        Save-Jpeg $png (Join-Path $Out "hero-$l-$m.jpg") 88
    }
}

# --- The buttons --------------------------------------------------------------------------
# All of a language's in a row on nothing, cut apart where nothing is drawn.
function Split-Row([string]$png, [string[]]$ids, [string]$name) {
    $bmp = [System.Drawing.Bitmap]::FromFile($png)
    try {
        $rect = New-Object System.Drawing.Rectangle 0, 0, $bmp.Width, $bmp.Height
        $data = $bmp.LockBits($rect, 'ReadOnly', 'Format32bppArgb')
        $bytes = New-Object byte[] ($data.Stride * $bmp.Height)
        [Runtime.InteropServices.Marshal]::Copy($data.Scan0, $bytes, 0, $bytes.Length)
        $bmp.UnlockBits($data)
        $cols = New-Object bool[] $bmp.Width
        $top = $bmp.Height; $bottom = -1
        for ($y = 0; $y -lt $bmp.Height; $y++) {
            $row = $y * $data.Stride
            for ($x = 0; $x -lt $bmp.Width; $x++) {
                if ($bytes[$row + $x * 4 + 3] -gt 0) {
                    $cols[$x] = $true
                    if ($y -lt $top) { $top = $y }
                    if ($y -gt $bottom) { $bottom = $y }
                }
            }
        }
        $runs = @()
        $start = -1
        for ($x = 0; $x -le $bmp.Width; $x++) {
            $on = $x -lt $bmp.Width -and $cols[$x]
            if ($on -and $start -lt 0) { $start = $x }
            if (-not $on -and $start -ge 0) { $runs += , @($start, $x); $start = -1 }
        }
        if ($runs.Count -ne $ids.Count) { throw "$png has $($runs.Count) buttons, not $($ids.Count)" }
        for ($i = 0; $i -lt $ids.Count; $i++) {
            $x0, $x1 = $runs[$i]
            $cut = $bmp.Clone((New-Object System.Drawing.Rectangle $x0, $top, ($x1 - $x0), ($bottom - $top + 1)), $bmp.PixelFormat)
            $cut.Save((Join-Path $Out ($name -f $ids[$i])), [System.Drawing.Imaging.ImageFormat]::Png)
            $cut.Dispose()
        }
    } finally { $bmp.Dispose() }
}
foreach ($l in $langs) {
    foreach ($m in $modes) {
        $png = Join-Path $work "buttons-$l-$m.png"
        Invoke-Shot "${page}?part=buttons&lang=$l&mode=$m" 1000 120 2 $png -Clear
        Split-Row $png $buttons[$l] "btn-{0}-$l-$m.png"
    }
}

$files = Get-ChildItem -LiteralPath $Out -File
'{0} pictures in {1}, {2:N0} KB' -f $files.Count, $Out, (($files | Measure-Object Length -Sum).Sum / 1KB)
