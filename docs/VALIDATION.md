# LanMix 0.1 验证记录

日期：2026-10-31。环境：Windows x64，Rust/Cargo 1.97.1。

## 自动检查

- `cargo fmt --all -- --check`：通过。
- `cargo clippy --locked --workspace --all-targets -- -D warnings`：通过；第三方 CPAL 原有两条编译警告仍在，未通过关闭项目检查来掩盖。
- `cargo test --locked --workspace`：37 项协议、DSP、网络和 JNI 参数/生命周期测试通过。
- `cargo test --manifest-path vendor/cpal/Cargo.toml --lib`：6 项通过，包含 4 项 WASAPI 静音缓冲补丁测试。
- `scripts/build-desktop.ps1`：Windows x64 release 构建与复制完成。

核心测试覆盖：增益和预放大、静音/独听、平衡、EQ 的频带响应、不同来源采样率、抗混叠、双声道独立状态、缓冲耗尽渐变、重置与重连、非有限输入和最终上限、处理路径零分配、小平台栈构造。时钟测试分别模拟正负 2000ppm 偏差各 60 秒，缓冲有界且未欠载；这是模拟时间，不是多台实体声卡的长时间运行报告。

网络测试覆盖：去重、乱序、交叠包、12ms 缺包截止时间、重排容量上限、容量满时优先接收当前缺口、旧会话隔离、旧格式包不能回退时间线、新设备静音门控、重连控制保留、槽位复用后参数重置。

## 真实进程与 UDP 测试

执行 `python scripts/smoke_test.py --bin-dir dist/windows-x64`：

| 项目 | 结果 |
|---|---:|
| 来源 | 两个实际 sender 进程，44.1kHz / 440Hz 和 96kHz / 660Hz |
| 接收端 | 实际 UDP receiver 和混音器，48kHz，无声 headless 模式 |
| 接收包 | 5817 |
| 来源 1 / 来源 2 包数 | 2506 / 3311 |
| 无效包 / 拒绝来源 | 0 / 0 |
| 缺失帧 / 迟到包 / 队列溢出 | 两路均为 0 |
| 输出峰值 | 约 0.050119 |
| 输出 RMS | 约 0.021131 |
| 输出削波帧 | 0 |

该测试将两路分别重采样并混音，检查输出有限、非静音且未越界。测试不播放声音，不能代替实体耳机听音或异机 Wi-Fi 验证。

Windows WASAPI 回环也已实际打开，通过采集队列向另一 headless 接收端发送了 1495 包，未出现协议错误、丢帧和队列溢出。该次检查采集的是静音运行的系统输出，并非音乐听感评价。静音标记补丁另有硬件无关的 NULL/旧指针/对齐/格式测试。

## 桌面界面

已验证窗口显示、启动接收端、打开系统默认输出流、读取设备格式信息、展开 10 段 EQ。随后用户按 Escape 停止了界面自动操作，之后未再控制桌面窗口。后续修复通过编译、单元和进程测试检查。

## Android 与跨平台边界

Android 使用原生 Kotlin Activity、AudioTrack 浮点写入线程和共享 Rust JNI；不是 Oboe 回调。提供 ARM64 和 ARMv7 安装包，构建和 JNI 检查详情见 `android/README.md`。APK 使用开发调试签名。

最终 APK 已重新构建，包含两种 ARM 架构和全部 6 个 JNI 导出；签名、16KB ELF 对齐和 ZIP 对齐检查通过。Android lint 为 0 个错误、48 条警告，主要涉及国际化、图标、备份配置及未包含的 ChromeOS 架构。真实 JVM `-Xcheck:jni` 测试通过，覆盖 UDP、静音门控、控制并发、数组校验和采样率重启；详细记录见 `android/BUILD-STATUS.md`。

**未进行 Android 真机安装、蓝牙/有线/USB 耳机听音、拔插与锁屏场景验证。** Android 当前在 Activity 可见时接收，转入后台会停止。物理硬件位深、蓝牙编码状态和链路音质不由应用流格式直接证明。

Linux/macOS 添加了 CI 检查配置，但本地未运行这些平台；macOS 系统声音采集和 iOS 应用外壳尚未实现。CI 配置文件的存在不代表远端 CI 已执行。自动发现、配对加密、后台服务和真正的前瞻限幅器也不包含在本版中。
