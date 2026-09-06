# SPDX-License-Identifier: Apache-2.0
#
# Draws site/public/og.png: the card that unfurls when somebody shares a link to
# cabalhunter.org.
#
# WHY A POWERSHELL SCRIPT IS IN A LINUX-FIRST REPOSITORY, AND WHY THAT IS FINE.
# Nothing in CI runs this. The PNG is committed, a test asserts its dimensions,
# and this file exists so the next person can see how the committed bytes were
# produced rather than treating them as a binary that appeared. Regeneration is
# by hand, on Windows, when the figures move -- which is the same cadence as the
# fixture the figures come from. If it ever needs to run on the build machine,
# rewrite it; do not add a headless browser to render a card with four strings
# on it.
#
# The figures are ARGUMENTS, not constants. They come from
# site/src/fixtures/stats.json, and passing them in is what stops this script
# becoming a second place where the site's numbers live. Read them out of the
# fixture and hand them over:
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/site/og.ps1 #     -Launches "528,871" -Graduate "2.87%" -Band "10.1x" -MeasuredOn "2026-09-05"
#
# -NoProfile is not optional on this workstation: the profile starts an
# interactive session-manager menu that reads stdin, and without the flag this
# script hangs at a prompt behind it rather than drawing anything.
#
# System.Drawing is used because it is present on Windows without installing
# anything. It is the least interesting part of this file.

[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)][string]$Launches,
  [Parameter(Mandatory = $true)][string]$Graduate,
  [Parameter(Mandatory = $true)][string]$Band,
  [Parameter(Mandatory = $true)][string]$MeasuredOn,
  [string]$Out
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

if (-not $Out) {
  $Out = Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) 'site/public/og.png'
}

# 1200x630 is the size every unfurler crops to. Anything else gets letterboxed
# or cropped by somebody else's rules.
$W = 1200
$H = 630

# The palette, matching index.css. OKLCH there, sRGB here, converted once and
# written down rather than recomputed: this file is not the place to reimplement
# a colour space.
$ink     = [System.Drawing.ColorTranslator]::FromHtml('#12141a')  # --color-ink
$line    = [System.Drawing.ColorTranslator]::FromHtml('#2b2f38')  # --color-line
$text    = [System.Drawing.ColorTranslator]::FromHtml('#eeeff2')  # --color-text
$dim     = [System.Drawing.ColorTranslator]::FromHtml('#a3a8b3')  # --color-dim
$faint   = [System.Drawing.ColorTranslator]::FromHtml('#7b8091')  # --color-faint
$signal  = [System.Drawing.ColorTranslator]::FromHtml('#f0b429')  # --color-signal

$bmp = New-Object System.Drawing.Bitmap($W, $H)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
# Grayscale antialiasing, NOT ClearType. ClearType renders sub-pixel colour
# fringes tuned for one particular LCD layout; this PNG is rescaled by whoever
# unfurls it, on screens whose subpixel order nobody here knows, and the fringes
# survive the rescale as coloured dirt on the letters. Caught by looking at the
# first render rather than by a check.
$g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
$g.Clear($ink)

try {
  # The plot grid, faint, matching the site's .grid-ground. It fades out down
  # the card by drawing fewer lines rather than by masking, because
  # System.Drawing has no mask and a gradient brush over a grid looks muddy.
  $pen = New-Object System.Drawing.Pen($line, 1)
  for ($x = 0; $x -lt $W; $x += 56) { $g.DrawLine($pen, $x, 0, $x, 330) }
  for ($y = 0; $y -lt 330; $y += 56) { $g.DrawLine($pen, 0, $y, $W, $y) }
  $pen.Dispose()

  $wordmarkFont = New-Object System.Drawing.Font('Consolas', 20, [System.Drawing.FontStyle]::Bold)
  $titleFont    = New-Object System.Drawing.Font('Bahnschrift', 62, [System.Drawing.FontStyle]::Bold)
  $figureFont   = New-Object System.Drawing.Font('Bahnschrift', 46, [System.Drawing.FontStyle]::Bold)
  $labelFont    = New-Object System.Drawing.Font('Segoe UI', 15)
  $footFont     = New-Object System.Drawing.Font('Segoe UI', 14)

  $brushText   = New-Object System.Drawing.SolidBrush($text)
  $brushDim    = New-Object System.Drawing.SolidBrush($dim)
  $brushFaint  = New-Object System.Drawing.SolidBrush($faint)
  $brushSignal = New-Object System.Drawing.SolidBrush($signal)

  # Wordmark.
  $g.DrawString('CABAL', $wordmarkFont, $brushText, 64, 56)
  $cabalW = $g.MeasureString('CABAL', $wordmarkFont).Width
  $g.DrawString('HUNTER', $wordmarkFont, $brushSignal, (64 + $cabalW - 8), 56)

  # The claim. Two lines, hand-broken: the break is a design decision and
  # letting a measuring loop choose it produces a worse one every time.
  $g.DrawString('Most launches are', $titleFont, $brushText, 56, 120)
  $g.DrawString('coordinated.', $titleFont, $brushSignal, 56, 196)

  $g.DrawString('You can see it before you buy.', $labelFont, $brushDim, 64, 288)

  # The three figures, on a rule.
  $pen = New-Object System.Drawing.Pen($line, 1)
  $g.DrawLine($pen, 56, 380, ($W - 56), 380)
  $pen.Dispose()

  $cols = @(
    @{ v = $Launches; l = 'launches watched'; c = $brushText },
    @{ v = $Graduate; l = 'ever graduate';    c = $brushSignal },
    @{ v = $Band;     l = 'more likely, at 10-13 recipients'; c = $brushSignal }
  )
  $x = 64
  foreach ($col in $cols) {
    $g.DrawString($col.v, $figureFont, $col.c, $x, 416)
    $g.DrawString($col.l, $labelFont, $brushDim, ($x + 4), 490)
    $x += 370
  }

  $g.DrawString(
    "Measured on $MeasuredOn. Measured, not predicted. Not financial advice.",
    $footFont, $brushFaint, 64, 556)

  $dir = Split-Path -Parent $Out
  if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
  $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
  Write-Output "wrote $Out ($W x $H)"
}
finally {
  $g.Dispose()
  $bmp.Dispose()
}
