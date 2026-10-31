param([string]$Destination = "dist/windows-x64")
$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    # A separate target keeps a running development executable from blocking linking.
    cargo build --locked --release --target-dir target/package -p lanmix-sender -p lanmix-receiver
    if ($LASTEXITCODE -ne 0) { throw "Rust build failed" }
    $outPath = [System.IO.Path]::GetFullPath((Join-Path $projectRoot $Destination))
    New-Item -ItemType Directory -Force -Path $outPath | Out-Null
    Copy-Item -LiteralPath (Join-Path $projectRoot "target/package/release/lanmix-sender.exe") -Destination $outPath
    Copy-Item -LiteralPath (Join-Path $projectRoot "target/package/release/lanmix.exe") -Destination $outPath
    Copy-Item -LiteralPath (Join-Path $projectRoot "target/package/release/lanmix-receiver.exe") -Destination $outPath
    Copy-Item -LiteralPath (Join-Path $projectRoot "README.md") -Destination $outPath
    Copy-Item -LiteralPath (Join-Path $projectRoot "docs/UI-DISCOVERY.md") -Destination (Join-Path $outPath "连接说明.md")
    Write-Output "Desktop binaries: $outPath"
} finally { Pop-Location }
