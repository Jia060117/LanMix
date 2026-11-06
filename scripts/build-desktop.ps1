param([string]$Destination = "dist/windows-x64-standalone")
$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    # A separate target keeps a running development executable from blocking linking.
    cargo build --locked --release --target-dir target/package -p lanmix-sender -p lanmix-receiver
    if ($LASTEXITCODE -ne 0) { throw "Rust build failed" }
    $outPath = [System.IO.Path]::GetFullPath((Join-Path $projectRoot $Destination))
    python scripts/package-desktop.py --platform windows-x64 --bin-dir target/package/release --output-dir $outPath
    if ($LASTEXITCODE -ne 0) { throw "Desktop staging failed" }
    Write-Output "Desktop binaries: $outPath"
} finally { Pop-Location }
