param (
    [string]$Version = ""
)

$ErrorActionPreference = "Stop"

# Detect version from Cargo.toml if not passed
if (-not $Version) {
    $cargoContent = Get-Content "Cargo.toml" -Raw
    if ($cargoContent -match 'version\s*=\s*"([^"]+)"') {
        $Version = $matches[1]
    } else {
        $Version = "0.2.0"
    }
}

Write-Host "==========================================" -ForegroundColor Cyan
Write-Host " Building ya.music-pro v$Version (Release) " -ForegroundColor Cyan
Write-Host "==========================================" -ForegroundColor Cyan

cargo build --release
if ($LASTEXITCODE -ne 0) {
    Write-Error "Release build failed."
    exit 1
}

$targetExe = "target\release\ya-music-pro.exe"
if (-not (Test-Path $targetExe)) {
    Write-Error "Binary not found at $targetExe"
    exit 1
}

$distDir = "dist"
if (-not (Test-Path $distDir)) {
    New-Item -ItemType Directory -Path $distDir | Out-Null
}

$zipName = "ya.music-pro-v$Version-windows-x64.zip"
$zipPath = Join-Path $distDir $zipName

if (Test-Path $zipPath) {
    Remove-Item $zipPath -Force
}

Write-Host "`nPackaging $targetExe -> $zipPath..." -ForegroundColor Yellow
Compress-Archive -Path $targetExe -DestinationPath $zipPath -Force

$hash = (Get-FileHash -Path $zipPath -Algorithm SHA256).Hash
$exeHash = (Get-FileHash -Path $targetExe -Algorithm SHA256).Hash

Write-Host "`n[SUCCESS] Release archive created!" -ForegroundColor Green
Write-Host "Archive:  $zipPath" -ForegroundColor White
Write-Host "SHA256:   $hash" -ForegroundColor Gray
Write-Host "Exe SHA:  $exeHash" -ForegroundColor Gray

Write-Host "`n==========================================" -ForegroundColor Cyan
Write-Host " GitHub Release Details (Copy & Paste):    " -ForegroundColor Cyan
Write-Host "==========================================" -ForegroundColor Cyan
Write-Host "Tag:     v$Version"
Write-Host "Title:   ya.music-pro v$Version - Audiophile Desktop Client"
Write-Host "Assets:  dist\$zipName (and/or target\release\ya-music-pro.exe)"
Write-Host "==========================================" -ForegroundColor Cyan
