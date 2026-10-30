# LanMix

🚧 正在施工中：多设备局域网音频聚合与混音工具，功能持续完善中。

把多台设备的声音，通过局域网汇聚到一个接收端。每台设备可以独立调节音量、静音、独听、左右平衡和 10 段均衡器，混音后统一输出到当前耳机、音箱或 USB 声卡。

**当前为 0.1 开发版。优先保证聚合、格式透明和可控混音，不承诺特定 CPU、内存或端到端延迟。**

## 已实现

- Rust Workspace：协议、实时 DSP、网络、平台音频、发送端、桌面接收端和 Android JNI。
- Windows WASAPI 系统声音回环采集；采集回调只写预分配 SPSC 队列，工作线程发送 UDP。
- 原始 f32 双声道 PCM，保留来源采样率，支持 8–192 kHz；每包不超过 1200 字节。
- 最多 16 路输入，按采样帧位置重排、去重和补偿缺失时间，独立断流和重连。
- 每路独立异步重采样和缓冲水位速率修正，预分配音频队列。
- 每路音量、静音、独听、平衡、EQ 预放大和 10 段 EQ；总音量和总 EQ。
- 桌面接收端图形界面、输出选择、系统默认输出跟随、系统报告的格式列表、应用打开的流格式、电平表。
- 设置保存在当前用户的配置目录，手动保存或退出时保存。
- Android 原生接收端：输出设备/格式查询、实际 AudioTrack 路由、音量与 EQ，详见 [android/README.md](android/README.md)。

**Android 0.1.2：** 新增均衡 / 稳定优先 / 响应优先 / 兼容四档播放模式，
请求较小的输出缓冲，出现输出缺粮时自动扩容。新增实际播放缓冲与排队估算、
中文界面、模式记忆及诊断复制。建议保留网络初始缓冲 **150ms**，先试默认均衡模式。
请求值不是端到端延迟；验证与边界见 [播放排队优化](docs/PLAYBACK-LATENCY.md)。

## Windows 快速开始

需要 Rust stable 和 MSVC C++ 构建工具；本机验证版本为 Rust 1.97.1。当前仓库的 `Cargo.lock` 固定依赖版本。

```powershell
cargo build --release -p lanmix-sender -p lanmix-receiver
.\target\release\lanmix-receiver.exe
```

在接收端点击 **Start receiver**。默认监听 UDP `46210`，新安装的图形接收端默认初始缓冲 150ms，已有设置保留。输出采用设备默认采样率；可以选择设备和工作采样率，点击 **Apply / restart** 应用。

**0.1.1 连续播放修复：** 每路缺粮后自动扩大缓冲目标，最高 400ms；缺包等待时间随缓冲调整，避免过早把迟到的音频替换成静音。界面显示实际缓冲、目标值和缺粮次数。升级后建议先把初始缓冲改为 **150ms** 再启动，详见 [断音诊断与验证](docs/AUDIO-DROPOUTS.md)。这会增加缓冲时间，优先保证连续播放。

在每台 Windows 发送设备上运行：

```powershell
.\lanmix-sender.exe devices
.\lanmix-sender.exe send --to 192.168.1.20:46210
```

把 IP 换成接收设备的局域网地址。默认采集当前系统默认输出端点；可用 `--output 1` 选择枚举出的输出设备。如果多个发送实例共用一个 IP，为它们指定不同的 `--stream-id`。

```powershell
.\lanmix-sender.exe send --to 192.168.1.20:46210 --output 1 --stream-id 2
```

局域网接收需要允许对应应用的 UDP 入站。程序不会自动修改系统防火墙。

也可以先发送低音量测试音：

```powershell
.\lanmix-sender.exe tone --to 127.0.0.1:46210 --frequency 440 --seconds 10
```

## 音频格式与音质

界面明确区分 **系统报告的能力** 与 **应用当前打开的流格式**。OS 共享混音器、蓝牙链路和外部硬件还可能执行转换。不能把 f32 的内部计算精度等同于耳机的真实 DAC 位深，也不能将多个格式列表的最大值拼成设备保证支持的组合。

内部处理和网络传输均为 f32，接收端按实际工作采样率混音。音量和 EQ 开关采用平滑处理。默认主音量 -6dB 留出叠加余量。最终 ±0.98 的保护目前是**硬上限**，触发时界面显示计数；它不是无失真的限幅器或 true-peak limiter。输入叠加或 EQ 提升较大时应降低预放大/主音量。

默认输出是双声道。单声道输出执行左右平均；多声道输出只向前两个声道写入，其他声道静音。发送端暂不自动把环绕声混成双声道，遇到多声道采集端点会明确拒绝。

## 平台边界

| 平台 | 当前状态 |
|---|---|
| Windows | 已实现系统声音发送和桌面接收；本机编译/验证记录见 `docs/VALIDATION.md` |
| Android | 已提供原生 Kotlin 接收端及 Rust JNI；构建和验证范围见 `android/README.md` |
| Linux | 桌面代码采用 CPAL/egui，可移植；系统声音发送需配置 monitor 输入并显式传 `--input`，未做本机运行验证 |
| macOS | 桌面接收架构可移植；系统声音采集的 Core Audio taps/ScreenCaptureKit 后端尚未实现 |
| iOS/iPadOS | 尚未提供应用外壳；共享 Rust 核心不等于已完成 iOS 产品 |

目前通过 IP 直接连接，设备显示为来源 IP 与 stream-id；不含自动发现、配对、加密、后台常驻服务、托盘和开机自启。协议 v1 面向可信局域网。这些产品能力不会以未生效的按钮冒充实现。

## 检查与构建

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo test --manifest-path vendor/cpal/Cargo.toml --lib
cargo clippy --workspace --all-targets -- -D warnings
python scripts/smoke_test.py --bin-dir target/release
powershell -ExecutionPolicy Bypass -File scripts/build-desktop.ps1
```

无音频设备的机器也可以运行真实 UDP 接收和混音验证，不会播放声音：

```powershell
.\target\release\lanmix-receiver.exe --headless --bind 127.0.0.1:46210 --seconds 10 --report report.json
```

## 项目结构

```text
crates/lanmix-proto      明确端序的协议与严格校验
crates/lanmix-core       重采样、时钟修正、EQ、音量与混音
crates/lanmix-network    UDP、重排、去重和来源生命周期
crates/lanmix-audio      桌面音频端点与格式、采集/播放支持
crates/lanmix-sender     CLI 采集发送和测试音
crates/lanmix-receiver   桌面界面和无声端到端测试模式
crates/lanmix-android    JNI 桥接
android/                Android 接收端
```

协议详情见 [crates/lanmix-proto/README.md](crates/lanmix-proto/README.md)。

Windows 采集依赖采用本地 CPAL 0.16.0 补丁，正确处理 WASAPI 静音缓冲标记。补丁范围与上游许可见 [vendor/cpal/LANMIX_PATCH.md](vendor/cpal/LANMIX_PATCH.md)。
