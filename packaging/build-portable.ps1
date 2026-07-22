param(
    [ValidateSet("debug", "release")]
    [string]$Configuration = "release",
    [string]$OutputDirectory = "dist",
    [string]$Version,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$workspace = Split-Path -Parent $PSScriptRoot
$output = [IO.Path]::GetFullPath((Join-Path $workspace $OutputDirectory))
$staging = Join-Path $output "portable-staging"
$archiveName = "termy-windows-x64.zip"
$archivePath = Join-Path $output $archiveName

$cargoManifest = Get-Content -Raw (Join-Path $workspace "Cargo.toml")
$versionMatch = [regex]::Match(
    $cargoManifest,
    '(?ms)^\[workspace\.package\]\s*.*?^version\s*=\s*"(?<version>\d+\.\d+\.\d+)"'
)
if (-not $versionMatch.Success) {
    throw "Could not read workspace.package.version from Cargo.toml"
}
$cargoVersion = $versionMatch.Groups["version"].Value
if ($Version -and $Version -ne $cargoVersion) {
    throw "Requested version $Version does not match Cargo.toml version $cargoVersion"
}
$Version = $cargoVersion

Push-Location $workspace
try {
    if (-not $SkipBuild) {
        cargo build --locked --profile $Configuration -p ade-app
        if ($LASTEXITCODE -ne 0) { throw "Cargo build failed" }
    }

    $sourceExecutable = Join-Path $workspace "target\$Configuration\ade-app.exe"
    if (-not (Test-Path -LiteralPath $sourceExecutable -PathType Leaf)) {
        throw "The Rust application was not found in target\$Configuration"
    }

    if (Test-Path -LiteralPath $staging) {
        Remove-Item -LiteralPath $staging -Recurse -Force
    }
    New-Item -ItemType Directory -Force $staging | Out-Null
    Copy-Item -LiteralPath $sourceExecutable -Destination (Join-Path $staging "termy.exe")
    Copy-Item -LiteralPath (Join-Path $workspace "README.md") -Destination $staging
    [IO.File]::WriteAllText(
        (Join-Path $staging "VERSION.txt"),
        "$Version`n",
        [Text.UTF8Encoding]::new($false)
    )

    New-Item -ItemType Directory -Force $output | Out-Null
    if (Test-Path -LiteralPath $archivePath) {
        Remove-Item -LiteralPath $archivePath -Force
    }
    Compress-Archive -Path (Join-Path $staging "*") -DestinationPath $archivePath -CompressionLevel Optimal

    $hash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText(
        "$archivePath.sha256",
        "$hash  $archiveName`n",
        [Text.UTF8Encoding]::new($false)
    )
    Write-Output "Created portable release: $archivePath"
} finally {
    if (Test-Path -LiteralPath $staging) {
        Remove-Item -LiteralPath $staging -Recurse -Force
    }
    Pop-Location
}
