param(
    [string]$Configuration = "release",
    [string]$OutputDirectory = "dist"
)

$ErrorActionPreference = "Stop"
$workspace = Split-Path -Parent $PSScriptRoot
$output = Join-Path $workspace $OutputDirectory
$staging = Join-Path $output "msix-staging"
$assets = Join-Path $staging "Assets"

Push-Location $workspace
try {
    cargo build --profile $Configuration -p ade-app -p ade-cli
    if ($LASTEXITCODE -ne 0) { throw "Cargo build failed" }

    Remove-Item -Recurse -Force $staging -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force $assets | Out-Null
    Copy-Item "target\$Configuration\ade-app.exe" $staging
    Copy-Item "target\$Configuration\ade-cli.exe" $staging
    Copy-Item "packaging\AppxManifest.xml" $staging

    Add-Type -AssemblyName System.Drawing
    function New-AdeAsset([string]$Path, [int]$Width, [int]$Height) {
        $source = [System.Drawing.Image]::FromFile((Join-Path $workspace "crates\ade-app\assets\app-icon.png"))
        $bitmap = [System.Drawing.Bitmap]::new($Width, $Height, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        try {
            $graphics.Clear([System.Drawing.Color]::Transparent)
            $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
            $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
            $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $side = [Math]::Min($Width, $Height)
            $left = [Math]::Floor(($Width - $side) / 2)
            $top = [Math]::Floor(($Height - $side) / 2)
            $graphics.DrawImage($source, $left, $top, $side, $side)
            $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
        } finally {
            $graphics.Dispose()
            $bitmap.Dispose()
            $source.Dispose()
        }
    }

    New-AdeAsset (Join-Path $assets "Square44x44Logo.png") 44 44
    New-AdeAsset (Join-Path $assets "Square150x150Logo.png") 150 150
    New-AdeAsset (Join-Path $assets "Wide310x150Logo.png") 310 150
    New-AdeAsset (Join-Path $assets "StoreLogo.png") 50 50

    $makeAppx = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Filter makeappx.exe -Recurse |
        Sort-Object FullName -Descending |
        Select-Object -First 1
    if (-not $makeAppx) { throw "makeappx.exe was not found in the Windows SDK" }

    New-Item -ItemType Directory -Force $output | Out-Null
    $package = Join-Path $output "ADE_0.1.0.0_x64.msix"
    & $makeAppx.FullName pack /d $staging /p $package /o
    if ($LASTEXITCODE -ne 0) { throw "makeappx failed" }

    Write-Output "Created unsigned MSIX: $package"
    Write-Output "Sign it with a certificate whose subject matches 'CN=ADE Development' before installation."
} finally {
    Pop-Location
}
