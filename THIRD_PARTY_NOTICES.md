# 开源许可与第三方声明

LanMix 原创代码采用 **MIT OR Apache-2.0** 双许可证，使用者可任选一种。
这与 Rust 工作区原有的许可声明保持一致，也便于其他项目复用；Apache-2.0
选项还包含明确的专利授权条款。第三方组件仍遵循各自许可证。

## 实际引用

| 组件 | 用途 | 许可 |
|---|---|---|
| [CPAL 0.16.0](https://github.com/RustAudio/cpal) | Windows WASAPI 回环采集与桌面音频输出 | Apache-2.0 |
| [egui / eframe 0.33.3](https://github.com/emilk/egui) | 桌面混音接收窗口 | MIT OR Apache-2.0 |
| [rtrb](https://github.com/mgeier/rtrb) | 音频环形队列 | MIT OR Apache-2.0 |
| [jni-rs](https://github.com/jni-rs/jni-rs) | Android 与 Rust 核心通信 | MIT OR Apache-2.0 |
| [socket2](https://github.com/rust-lang/socket2) | 网络套接字 | MIT OR Apache-2.0 |
| [if-addrs](https://github.com/messense/if-addrs) | 局域网网卡枚举 | MIT OR BSD-3-Clause |
| [Serde](https://github.com/serde-rs/serde)、[Clap](https://github.com/clap-rs/clap)、[anyhow](https://github.com/dtolnay/anyhow) | 配置、命令行与错误处理 | MIT OR Apache-2.0 |
| [Kotlin stdlib 2.2.10](https://github.com/JetBrains/kotlin) | Android Kotlin 运行库 | Apache-2.0 |
| [JetBrains annotations 13.0](https://github.com/JetBrains/intellij-community) | Kotlin 运行库的注解依赖 | Apache-2.0 |

`vendor/cpal` 保留上游 Apache-2.0 许可，并含 LanMix 的 WASAPI 静音缓冲补丁。
修改说明见 [LANMIX_PATCH.md](vendor/cpal/LANMIX_PATCH.md)；该目录及补丁按
Apache-2.0 分发，不因项目根目录的双许可证而改变授权。

桌面 `epaint_default_fonts` 内嵌的字体须额外保留 **SIL OFL-1.1、Ubuntu Font
Licence 1.0**、Hack 的 MIT / Bitstream Vera 条款及 emoji 字体 MIT 声明。字体不按 LanMix 的双许可证重新授权。
完整字体版权和许可文本收录在 [THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt)。

## 清单与核查范围

- [Rust 依赖清单](third-party/rust-dependencies.json)：187 个外部包，按 Windows x64、Android ARM64 / ARMv7 的锁定依赖图收集，含构建依赖，排除测试专用边。它是保守声明清单，不声称每个依赖都链接进二进制。
- [完整许可文本](THIRD_PARTY_LICENSES.txt)：保留各包随附的许可、版权、NOTICE 和字体条款，附 Android 运行库声明。
- [上游补充来源](third-party/upstream-sources.json) 与 [Android 来源](third-party/android-sources.json)：缺失的文本从对应提交、发布标签或版本化 Maven 制品补齐，缓存于 `third-party/upstream/`。
- 本次正式二进制使用 Rust 1.97.1，其工具链随附的 [标准库版权与许可](third-party/rust-standard-library-1.97.1.html) 也完整保留，并收入许可全文。该清单覆盖标准库的多平台来源，不表示每个条目都链接到本次应用；更换发布工具链时须同步更新。
- 依赖更新后运行 `python scripts/collect-licenses.py` 重新生成；打包脚本会检查清单对应的 `Cargo.lock` 摘要。
- Rust 存在多选一授权时可采用其中兼容的宽松许可；字体的 `AND` 许可需要同时遵守。本次 Windows / Android 范围没有发现必须以 GPL / LGPL 授权 LanMix 原创代码的依赖。

JUnit、Robolectric 用于测试；Gradle、Android Gradle Plugin、Kotlin 编译器、JDK、
Android SDK / NDK 与 GitHub Actions 用于构建，不作为应用本身的授权协议。
本清单重点覆盖正式 Windows / Android 下载，其他平台发布前仍需核对对应依赖。

## 视觉参考

Android 界面参考了 [KernelSU Manager](https://github.com/tiann/KernelSU) 的简洁布局
和浮动导航思路。当前实现使用原生 Kotlin Views、Canvas 和 AGSL，未复制其源码、
图标或资源，也未引入 Miuix 或 Compose。这里的致谢不代表两项目存在官方关联。
初期方案提及的 Oboe、rubato、bytemuck 与 Opus 没有作为当前实现的直接依赖使用。

Windows 两个独立 EXE 均内置压缩的项目与第三方许可全文，可用 `--licenses 文件路径`
导出；Android APK 在 assets 中保留许可文本。重新分发时请继续保留相关版权、
许可证和必要声明。
