# 源码构建与正式打包

正式发行范围为 Windows x64 和 Android ARM64 / ARMv7。Linux、macOS 的 CI
产物是开发构建；iOS 尚无 App 工程。检查通过不等于实体设备听音验收。

## Windows

需要 Rust / MSVC C++ 工具链和 Python 3.11+。CI 固定 Rust 1.99.1；本次本地
验证另有 Rust 1.97.1。使用仓库的 Cargo.lock，不要为了发布刷新全部依赖。

```powershell
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo test --manifest-path vendor/cpal/Cargo.toml --lib
cargo build --locked --release -p lanmix-sender -p lanmix-receiver
python scripts/smoke_test.py --bin-dir target/release
python scripts/test_redundancy.py --bin-dir target/release
python scripts/test_discovery.py --bin-dir target/release
python scripts/package-desktop.py --platform windows-x64
```

打包脚本检查本机架构、程序版本和许可清单对应的 Cargo.lock 摘要，并写入提交号。
生成物在被 Git 忽略的 `dist/`。重新发布前应对解压后的实际二进制重做进程测试。
Windows 程序当前没有商业代码签名。

## 许可清单

```powershell
python scripts/collect-licenses.py
```

需要锁定依赖已下载到 Cargo 缓存。脚本使用离线 Cargo 元数据；少数缺失的上游
许可按固定提交从网络补齐，成功结果缓存到 `third-party/upstream/`。Android
运行库的版本化来源列于 `third-party/android-sources.json`，升级时需同步核对。

## Android

本机构建环境：JDK 21、Android SDK 37、NDK 30.0.14904198、Gradle 9.6.1、
Android Gradle Plugin 9.3.2。Rust 需要 `aarch64-linux-android` 和
`armv7-linux-androideabi` 两个 target。使用 JDK 21 运行测试。

```powershell
$env:JAVA_HOME = '你的 JDK 21 目录'
$env:ANDROID_HOME = '你的 Android SDK 目录'
.\android\build.ps1 -Offline
.\android\verify-jni.ps1
```

脚本先构建双 ABI JNI、复制项目及第三方许可到 APK assets，再运行 Debug 单元测试、
构建和 lint。JNI 宿主验证通过实际 JVM `-Xcheck:jni` 检查导出函数与 UDP 链路。
`-SkipNative` 仅用于确认现有 JNI 与当前源码一致时复用，正式发布应重建 JNI。

### 正式签名

v1.0.0 开始使用 Jia060117 的专用发布密钥。密钥与口令只能保存在仓库外，
不得提交 Git、上传 Release 或输出至日志。后续正式版必须使用相同密钥。

在当前进程中安全设置以下环境变量，再运行构建：

| 变量 | 内容 |
|---|---|
| `LANMIX_KEYSTORE` | 仓库外的 JKS 文件路径 |
| `LANMIX_STORE_PASSWORD` | 密钥库口令 |
| `LANMIX_KEY_ALIAS` | 别名，默认 `lanmix` |
| `LANMIX_KEY_PASSWORD` | 私钥口令 |

```powershell
.\android\build.ps1 -Release -Offline
```

增加 Release 单元测试、构建和 lint；缺少发布签名参数时直接失败。
输出 `android/app/build/outputs/apk/release/app-release.apk`。使用 SDK 工具检查：

```text
apksigner verify --verbose --print-certs app-release.apk
zipalign -c -P 16 -v 4 app-release.apk
aapt2 dump badging app-release.apk
```

还需确认非 debuggable、双 ARM 架构、许可 assets、版本号与证书指纹。
旧调试 APK 不能直接覆盖安装正式 APK，需卸载；之后的正式版可同签名升级。

## 发布

相关检查通过后提交源码，再基于该提交打包、标记版本，上传 Windows ZIP、
Android 正式签名 APK 和 SHA256SUMS.txt。正式 Release 不上传其他平台开发产物。
测试边界和未完成的实体设备验收必须如实保留在验证记录中。
