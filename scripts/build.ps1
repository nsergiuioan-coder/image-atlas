param([string]$Nsis = 'makensis')
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    cargo fmt --check
    if ($LASTEXITCODE) { throw 'Formatting check failed' }
    cargo test --locked --release
    if ($LASTEXITCODE) { throw 'Tests failed' }
    cargo clippy --locked --release -- -D warnings
    if ($LASTEXITCODE) { throw 'Clippy failed' }
    cargo build --locked --release
    if ($LASTEXITCODE) { throw 'Build failed' }
    python scripts/attributions.py
    if ($LASTEXITCODE) { throw 'Attribution generation failed' }
    $version = ((Get-Content Cargo.toml | Select-String '^version = ').Line -split '"')[1]
    New-Item -ItemType Directory -Force dist | Out-Null
    & $Nsis "/DVERSION=$version" packaging/windows.nsi
    if ($LASTEXITCODE) { throw 'Installer build failed' }
    $portable = Join-Path $projectRoot 'dist/portable'
    New-Item -ItemType Directory -Force $portable | Out-Null
    Copy-Item target/release/image-atlas.exe "$portable/ImageAtlas.exe"
    Copy-Item LICENSE,NOTICE,ATTRIBUZIONI.txt,README.md $portable
    Copy-Item THIRD-PARTY-LICENSES $portable -Recurse -Force
    Compress-Archive -Path "$portable/*" -DestinationPath "dist/ImageAtlas-$version-portable.zip" -Force
    Get-Item "dist/ImageAtlas-$version-setup.exe", "dist/ImageAtlas-$version-portable.zip" | ForEach-Object { '{0}  {1}' -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.Name } | Set-Content dist/SHA256SUMS.txt
} finally { Pop-Location }
