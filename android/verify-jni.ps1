$ErrorActionPreference = 'Stop'
$workspaceDir = Split-Path -Parent $PSScriptRoot
$jdkDir = if ($env:JAVA_HOME) { $env:JAVA_HOME } else { 'C:\Program Files\Android\Android Studio\jbr' }
$targetDir = Join-Path $PSScriptRoot '.verification\target'
$classesDir = Join-Path $PSScriptRoot '.verification\classes'
& cargo build --manifest-path (Join-Path $workspaceDir 'crates\lanmix-android\Cargo.toml') --lib --target-dir $targetDir --offline
if ($LASTEXITCODE -ne 0) { throw 'Host JNI DLL build failed.' }
New-Item -ItemType Directory -Force -Path $classesDir | Out-Null
& (Join-Path $jdkDir 'bin\javac.exe') -d $classesDir (Join-Path $PSScriptRoot 'verification\NativeBridge.java')
if ($LASTEXITCODE -ne 0) { throw 'JNI verification Java compilation failed.' }
& (Join-Path $jdkDir 'bin\java.exe') -Xcheck:jni -cp $classesDir io.lanmix.receiver.NativeBridge (Join-Path $targetDir 'debug\lanmix_android.dll')
if ($LASTEXITCODE -ne 0) { throw 'Host JVM JNI verification failed.' }
