param(
    [ValidateSet("debug", "release")]
    [string]$Configuration = "release",
    [string]$OutputDirectory = "dist",
    [string]$Version,
    [string]$Publisher,
    [string]$CertificatePath,
    [string]$CertificatePassword = $env:WINDOWS_SIGNING_CERTIFICATE_PASSWORD,
    [string]$TimestampUrl = "http://timestamp.digicert.com",
    [string]$PackageUri,
    [string]$AppInstallerUri,
    [switch]$StableFileNames,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$workspace = Split-Path -Parent $PSScriptRoot
$output = [System.IO.Path]::GetFullPath((Join-Path $workspace $OutputDirectory))
$staging = Join-Path $output "msix-staging"
$assets = Join-Path $staging "Assets"
$manifestTemplate = Join-Path $PSScriptRoot "AppxManifest.xml"
$packageName = "Termy.TerminalWorkspace"

function Get-WorkspaceVersion {
    $cargoManifest = Get-Content -Raw (Join-Path $workspace "Cargo.toml")
    $match = [regex]::Match(
        $cargoManifest,
        '(?ms)^\[workspace\.package\]\s*.*?^version\s*=\s*"(?<version>\d+\.\d+\.\d+)"'
    )
    if (-not $match.Success) {
        throw "Could not read workspace.package.version from Cargo.toml"
    }
    return $match.Groups["version"].Value
}

function ConvertTo-MsixVersion([string]$SemanticVersion) {
    if ($SemanticVersion -notmatch '^(\d+)\.(\d+)\.(\d+)$') {
        throw "Version '$SemanticVersion' must use major.minor.patch numeric notation"
    }
    $parts = @([int]$Matches[1], [int]$Matches[2], [int]$Matches[3], 0)
    if ($parts | Where-Object { $_ -gt 65535 }) {
        throw "Each MSIX version component must be between 0 and 65535"
    }
    return $parts -join "."
}

function Find-WindowsSdkTool([string]$ToolName) {
    $sdkBin = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
    $tool = Get-ChildItem $sdkBin -Filter $ToolName -Recurse -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match '\\x64\\' } |
        Sort-Object FullName -Descending |
        Select-Object -First 1
    if (-not $tool) {
        throw "$ToolName was not found. Install the Windows 10/11 SDK."
    }
    return $tool.FullName
}

function Invoke-SignTool([string]$SignTool, [string]$FilePath) {
    $arguments = @(
        "sign", "/fd", "SHA256", "/f", $CertificatePath,
        "/p", $CertificatePassword
    )
    if ($TimestampUrl) {
        $arguments += @("/tr", $TimestampUrl, "/td", "SHA256")
    }
    $arguments += $FilePath
    & $SignTool $arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Signing failed for $FilePath"
    }
}

if (-not $Version) {
    $Version = Get-WorkspaceVersion
}
$packageVersion = ConvertTo-MsixVersion $Version

if ([bool]$PackageUri -ne [bool]$AppInstallerUri) {
    throw "PackageUri and AppInstallerUri must be supplied together"
}
if (($PackageUri -or $AppInstallerUri) -and -not $CertificatePath) {
    throw "Auto-updating distribution packages must be signed"
}

$signTool = $null
if ($CertificatePath) {
    $CertificatePath = [System.IO.Path]::GetFullPath($CertificatePath)
    if (-not (Test-Path -LiteralPath $CertificatePath -PathType Leaf)) {
        throw "Signing certificate was not found at $CertificatePath"
    }
    if (-not $CertificatePassword) {
        throw "CertificatePassword or WINDOWS_SIGNING_CERTIFICATE_PASSWORD is required"
    }

    $certificate = [System.Security.Cryptography.X509Certificates.X509Certificate2]::new(
        $CertificatePath,
        $CertificatePassword
    )
    try {
        if ($Publisher -and $Publisher -ne $certificate.Subject) {
            throw "Publisher '$Publisher' does not match certificate subject '$($certificate.Subject)'"
        }
        $Publisher = $certificate.Subject
    } finally {
        $certificate.Dispose()
    }
    $signTool = Find-WindowsSdkTool "signtool.exe"
} elseif (-not $Publisher) {
    $Publisher = "CN=Termy Development"
}

Push-Location $workspace
try {
    if (-not $SkipBuild) {
        cargo build --locked --profile $Configuration -p ade-app
        if ($LASTEXITCODE -ne 0) { throw "Cargo build failed" }
    }

    $appExecutable = Join-Path $workspace "target\$Configuration\ade-app.exe"
    if (-not (Test-Path $appExecutable)) {
        throw "The Rust application was not found in target\$Configuration"
    }

    if (Test-Path -LiteralPath $staging) {
        Remove-Item -LiteralPath $staging -Recurse -Force
    }
    New-Item -ItemType Directory -Force $assets | Out-Null
    Copy-Item $appExecutable (Join-Path $staging "ade-app.exe")
    Copy-Item $manifestTemplate (Join-Path $staging "AppxManifest.xml")

    [xml]$manifest = Get-Content -Raw (Join-Path $staging "AppxManifest.xml")
    $manifest.Package.Identity.Version = $packageVersion
    $manifest.Package.Identity.Publisher = $Publisher
    $utf8WithoutBom = [System.Text.UTF8Encoding]::new($false)
    [System.IO.File]::WriteAllText(
        (Join-Path $staging "AppxManifest.xml"),
        $manifest.OuterXml,
        $utf8WithoutBom
    )

    Add-Type -AssemblyName System.Drawing
    function New-TermyAsset([string]$Path, [int]$Width, [int]$Height) {
        $source = [System.Drawing.Image]::FromFile(
            (Join-Path $workspace "crates\ade-app\assets\app-icon.png")
        )
        $bitmap = [System.Drawing.Bitmap]::new(
            $Width,
            $Height,
            [System.Drawing.Imaging.PixelFormat]::Format32bppArgb
        )
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

    New-TermyAsset (Join-Path $assets "Square44x44Logo.png") 44 44
    New-TermyAsset (Join-Path $assets "Square150x150Logo.png") 150 150
    New-TermyAsset (Join-Path $assets "Wide310x150Logo.png") 310 150
    New-TermyAsset (Join-Path $assets "StoreLogo.png") 50 50

    if ($signTool) {
        Invoke-SignTool $signTool (Join-Path $staging "ade-app.exe")
    }

    $makeAppx = Find-WindowsSdkTool "makeappx.exe"
    New-Item -ItemType Directory -Force $output | Out-Null
    $packageFileName = if ($StableFileNames) {
        "termy-x64.msix"
    } else {
        "termy_${packageVersion}_x64.msix"
    }
    $package = Join-Path $output $packageFileName
    & $makeAppx pack /d $staging /p $package /o
    if ($LASTEXITCODE -ne 0) { throw "makeappx failed" }

    if ($signTool) {
        Invoke-SignTool $signTool $package
        & $signTool verify /pa /v $package
        if ($LASTEXITCODE -ne 0) { throw "Signature verification failed for $package" }
    }

    if ($PackageUri -and $AppInstallerUri) {
        $escapedPublisher = [System.Security.SecurityElement]::Escape($Publisher)
        $escapedPackageUri = [System.Security.SecurityElement]::Escape($PackageUri)
        $escapedAppInstallerUri = [System.Security.SecurityElement]::Escape($AppInstallerUri)
        $appInstaller = @"
<?xml version="1.0" encoding="utf-8"?>
<AppInstaller xmlns="http://schemas.microsoft.com/appx/appinstaller/2021"
  Version="$packageVersion"
  Uri="$escapedAppInstallerUri">
  <MainPackage
    Name="$packageName"
    Publisher="$escapedPublisher"
    Version="$packageVersion"
    ProcessorArchitecture="x64"
    Uri="$escapedPackageUri" />
  <UpdateSettings>
    <OnLaunch HoursBetweenUpdateChecks="0" ShowPrompt="false" UpdateBlocksActivation="false" />
    <AutomaticBackgroundTask />
  </UpdateSettings>
</AppInstaller>
"@
        $appInstallerPath = Join-Path $output "termy.appinstaller"
        [System.IO.File]::WriteAllText($appInstallerPath, $appInstaller, $utf8WithoutBom)
        Write-Output "Created update feed: $appInstallerPath"
    }

    $checksumPath = "$package.sha256"
    $hash = (Get-FileHash -Algorithm SHA256 $package).Hash.ToLowerInvariant()
    [System.IO.File]::WriteAllText(
        $checksumPath,
        "$hash  $packageFileName`n",
        $utf8WithoutBom
    )

    if ($signTool) {
        Write-Output "Created signed MSIX: $package"
    } else {
        Write-Warning "Created unsigned development MSIX: $package"
        Write-Warning "Unsigned packages are not suitable for distribution."
    }
} finally {
    Pop-Location
}
