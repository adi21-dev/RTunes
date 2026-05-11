# Build release binary and stage dist/rtunes-v<ver>-<platform>/ for zipping.
$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $RepoRoot

$cargoToml = Join-Path $RepoRoot "Cargo.toml"
$versionLine = Select-String -Path $cargoToml -Pattern '^version\s*=\s*"(.+)"' | Select-Object -First 1
if (-not $versionLine) { throw "Could not parse version from Cargo.toml" }
$Version = $versionLine.Matches.Groups[1].Value

$hostTriple = (& rustc -vV | Select-String "^host: ").ToString().Replace("host: ", "").Trim()
$plat = $hostTriple -replace "[^a-zA-Z0-9_-]+", "_"

cargo build --release

$WinExe = Join-Path $RepoRoot "target/release/rtunes.exe"
$NixExe = Join-Path $RepoRoot "target/release/rtunes"
if (Test-Path $WinExe) { $ExeName = "rtunes.exe"; $Bin = $WinExe }
elseif (Test-Path $NixExe) { $ExeName = "rtunes"; $Bin = $NixExe }
else { throw "Missing release binary under target/release/ — build failed?" }

$DistName = "rtunes-v$Version-$plat"
$Stage = Join-Path $RepoRoot "dist/$DistName"
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
Copy-Item -Force $Bin (Join-Path $Stage $ExeName)

$Readme = Join-Path $RepoRoot "README.md"
if (Test-Path $Readme) { Copy-Item -Force $Readme (Join-Path $Stage "README.md") }

$DepsDir = Join-Path $Stage "deps"
New-Item -ItemType Directory -Force -Path $DepsDir | Out-Null
@"
Place yt-dlp and ffmpeg here (or install to PATH).
See README.md -> deps/ folder.
"@ | Set-Content -Encoding UTF8 (Join-Path $DepsDir "README.txt")

$Zip = Join-Path $RepoRoot "dist/$DistName.zip"
if (Test-Path $Zip) { Remove-Item -Force $Zip }
Compress-Archive -Path $Stage -DestinationPath $Zip
Write-Host "Staged: $Stage"
Write-Host "Zip:    $Zip"
