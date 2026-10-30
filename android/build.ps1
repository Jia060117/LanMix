[CmdletBinding()]
param([switch]$SkipNative, [switch]$Offline)
$ErrorActionPreference = 'Stop'
$projectDir = $PSScriptRoot
$workspaceDir = Split-Path -Parent $projectDir

if (-not $env:ANDROID_HOME) {
    $env:ANDROID_HOME = Join-Path $env:LOCALAPPDATA 'Android\Sdk'
}
if (-not (Test-Path -LiteralPath $env:ANDROID_HOME)) { throw 'Set ANDROID_HOME to the installed Android SDK.' }
if (-not $env:JAVA_HOME) {
    $bundledJdk = 'C:\Program Files\Android\Android Studio\jbr'
    if (Test-Path -LiteralPath $bundledJdk) { $env:JAVA_HOME = $bundledJdk }
}
$ndkDir = Join-Path $env:ANDROID_HOME 'ndk\30.0.14904198'
$toolchain = Join-Path $ndkDir 'toolchains\llvm\prebuilt\windows-x86_64\bin'
if (-not (Test-Path -LiteralPath $toolchain)) { throw "Installed NDK not found: $ndkDir" }

if (-not $SkipNative) {
    $targets = @(
        @{ Rust = 'aarch64-linux-android'; Abi = 'arm64-v8a'; Compiler = 'aarch64-linux-android26-clang.cmd'; Key = 'AARCH64_LINUX_ANDROID' },
        @{ Rust = 'armv7-linux-androideabi'; Abi = 'armeabi-v7a'; Compiler = 'armv7a-linux-androideabi26-clang.cmd'; Key = 'ARMV7_LINUX_ANDROIDEABI' }
    )
    $installed = & rustup target list --installed
    if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect installed Rust targets.' }
    $nativeTargetDir = Join-Path $projectDir '.native-target'
    foreach ($target in $targets) {
        if ($target.Rust -notin $installed) { throw "Rust target $($target.Rust) must be installed first." }
        $linker = Join-Path $toolchain $target.Compiler
        if (-not (Test-Path -LiteralPath $linker)) { throw "Missing NDK linker: $linker" }
        $linkerKey = "CARGO_TARGET_$($target.Key)_LINKER"
        $rustFlagsKey = "CARGO_TARGET_$($target.Key)_RUSTFLAGS"
        $oldLinker = [Environment]::GetEnvironmentVariable($linkerKey, 'Process')
        $oldFlags = [Environment]::GetEnvironmentVariable($rustFlagsKey, 'Process')
        try {
            [Environment]::SetEnvironmentVariable($linkerKey, $linker, 'Process')
            [Environment]::SetEnvironmentVariable($rustFlagsKey, '-C link-arg=-Wl,-z,max-page-size=16384', 'Process')
            $cargoArgs = @('build', '--manifest-path', (Join-Path $workspaceDir 'crates\lanmix-android\Cargo.toml'),
                '--lib', '--release', '--target', $target.Rust, '--target-dir', $nativeTargetDir)
            if ($Offline) { $cargoArgs += '--offline' }
            & cargo @cargoArgs
            if ($LASTEXITCODE -ne 0) { throw "Rust build failed for $($target.Rust)" }
            $destination = Join-Path $projectDir "app\src\main\jniLibs\$($target.Abi)"
            New-Item -ItemType Directory -Force -Path $destination | Out-Null
            Copy-Item -LiteralPath (Join-Path $nativeTargetDir "$($target.Rust)\release\liblanmix_android.so") -Destination $destination
        } finally {
            [Environment]::SetEnvironmentVariable($linkerKey, $oldLinker, 'Process')
            [Environment]::SetEnvironmentVariable($rustFlagsKey, $oldFlags, 'Process')
        }
    }
}

# Prefer the existing installed Gradle distribution. Never install Gradle/SDKs.
$cachedGradle = Get-ChildItem -LiteralPath (Join-Path $env:USERPROFILE '.gradle\wrapper\dists\gradle-9.6.1-bin') -Recurse -Filter gradle.bat -ErrorAction SilentlyContinue | Select-Object -First 1
$gradle = if ($cachedGradle) { $cachedGradle.FullName } else { Join-Path $projectDir 'gradlew.bat' }
if (-not (Test-Path -LiteralPath $gradle)) { throw 'Gradle 9.6.1 cache or Gradle wrapper is required.' }
$gradleArgs = @('-p', $projectDir, 'assembleDebug', 'lintDebug', '--console=plain')
if ($Offline) { $gradleArgs += '--offline' }
& $gradle @gradleArgs
if ($LASTEXITCODE -ne 0) { throw 'Android build or lint failed.' }
Write-Output "APK: $(Join-Path $projectDir 'app\build\outputs\apk\debug\app-debug.apk')"
