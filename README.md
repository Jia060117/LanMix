<div align="center">

# LanMix

**让每台设备的声音，都有一个共同的出口。**

把多台电脑的声音汇聚到手机或电脑，独立调节，统一聆听。

[![License: MIT / Apache-2.0](https://img.shields.io/badge/License-MIT%20%2F%20Apache--2.0-blue.svg)](LICENSE)
[![Platform: Windows / Android](https://img.shields.io/badge/Platform-Windows%20%7C%20Android-607D8B.svg)](#运行环境与兼容说明)
[![GitHub Release](https://img.shields.io/github/v/release/Jia060117/LanMix)](https://github.com/Jia060117/LanMix/releases/latest)
[![Rust desktop checks](https://github.com/Jia060117/LanMix/actions/workflows/check.yml/badge.svg)](https://github.com/Jia060117/LanMix/actions/workflows/check.yml)

由 **[情绪零碎の佳 · Jia060117](https://github.com/Jia060117)** 开发与维护

[下载正式版](https://github.com/Jia060117/LanMix/releases/latest) · [运行环境](#运行环境与兼容说明) · [使用方法](#开始使用) · [源码构建](#开发与构建) · [反馈问题](https://github.com/Jia060117/LanMix/issues)

</div>

耳机连接手机时，也想听到台式机、笔记本上的声音？LanMix 通过局域网接收这些设备的
系统音频，在接收端混音，再交给当前耳机、音箱或 USB 声卡播放。

**v1.0.0 正式版提供 Windows x64 和 Android 安装包。** 项目以多设备聚合、
音质和稳定播放为优先，持续完善界面与设备兼容性。

当前 Windows / Android 源码为 **1.0.3**：Windows 增加按钮反馈、连接状态卡片和
托盘连接 / 断开 / 重连；Android 在连接时就显示设备，无需等到电脑出声。
沿用单一调音编辑器和 EQ 预设。本轮更新尚未替换正式下载。

## 能做什么

| 功能 | 说明 |
|---|---|
| 多设备聚合 | 最多 16 路来源，各路独立接入、断开和重连 |
| 局域网发现 | 两端自动发现设备、点击连接，也可手动输入 IP |
| Windows 托盘 | 1.0.3 起关闭 / 最小化收起到托盘，右键连接、断开、重连或退出 |
| 独立混音 | 单设备音量滑块、静音、独听、左右平衡，以及总音量 |
| 10 段均衡器 | 每路和总输出独立设置；Android 提供横排推子、6 种预设与预放大 |
| 音质优先 | 不做有损编码，使用浮点立体声 PCM，保留来源采样率并独立重采样 |
| 输出信息 | 查看系统报告的设备能力、应用打开的流格式与实际输出路由 |
| Android 后台 | 前台服务持续接收，不主动申请音频焦点，可与其他共享输出应用共播 |
| 故障诊断 | 电平、缓冲、网络缺帧和输出缺粮统计，支持复制诊断信息 |

Windows 发送窗口使用 Rust / Win32，无 WebView。Android 使用原生 Kotlin 界面，
提供混音、设备、设置三页，随系统切换深浅色，浮动玻璃底栏按系统绘图能力降级。

## 运行环境与兼容说明

| 平台 | 正式下载 | 说明 |
|---|---|---|
| Windows 10 / 11 x64 | `LanMix-1.0.0-windows-x64.zip` | 原生发送窗口、命令行发送端、桌面混音接收端 |
| Android 8.0+ / API 26+ | `LanMix-1.0.0-android.apk` | ARM64 / ARMv7 双架构，无需 Root；Android 17+ 需允许本地网络访问 |
| Linux / macOS | 暂不提供正式版 | 有 CI 开发构建，尚未完成实体音频设备验收；macOS 系统声音采集未实现 |
| iOS / iPadOS | 暂不支持 | 尚无可安装 App 工程 |

设备需处于可互通的局域网。默认音频端口为 **UDP 46210**，自动发现使用
**UDP 46211**，连接控制另用动态 UDP 端口。访客网络、AP 隔离、VPN 或防火墙
可能影响发现和音频收发。

正式包沿用已迭代验证的 Windows / Android 功能。自动测试、进程联机测试和
此前真机验证的范围见 [验证记录](docs/VALIDATION.md)，不代表所有手机、GPU、
耳机与路由器组合均已验收。

## 开始使用

### 1. 下载并打开

从 [Releases](https://github.com/Jia060117/LanMix/releases/latest) 下载正式包。
Windows 解压后双击 **`lanmix.exe`**；手机安装 APK 后打开 LanMix。
可用同页 `SHA256SUMS.txt` 核对下载文件。

**从旧测试版升级：** v1.0.0 改用 Jia060117 专用发布签名。此前调试签名 APK
需先卸载再安装，旧版应用设置会清除。后续正式版沿用该发布密钥。
Windows 程序未购买商业代码签名证书。

### 2. 连接设备

1. 手机与电脑接入同一局域网，允许相关网络权限和防火墙访问。
2. 手机进入「设备」，选择电脑，点击「连接并接收」。Windows 保持「允许接收端发起连接」开启。
3. 或先在手机「混音」页开始接收，再从 Windows 设备列表选择手机并连接。
4. 其他电脑重复连接同一接收端，即可同时听到多台设备。

1.0.3 两端配套使用时，Android 会显示「正在连接 → 已连接 · 等待电脑声音 → 接收中」。
Windows 关闭窗口或最小化后继续发送，点击托盘图标恢复；完全退出请右键托盘选择「退出 LanMix」，
也可从窗口系统菜单退出。设置只记住上次成功连接，不会在启动时自动推送声音。

如果要让电脑作为混音中心，运行 **`lanmix-receiver.exe`**。
发现失败时可手动输入接收端地址；命令行示例：

```powershell
.\lanmix-sender.exe send --to 192.168.1.184:46210
```

多台电脑可直接连接同一接收端；同一台电脑启动多个 CLI 来源时，请使用不同的 `--stream-id`。

### 3. 调整听感

Android 1.0.1 及更新版本的「混音」页只保留一处「调音」：先选择「整体」或某台设备，再调整
共用的音量滑块；选中单台设备时可设置静音与独听。展开「音效」，可使用横排的
10 段竖向推子，或选择平直、低音增强、人声、流行、摇滚、古典预设。每个调音对象
的设置独立保留；预放大收在「高级 · 输出余量」。

v1.0.0 接收端仍在各音源卡片中调整音量及均衡器。输出设备、实际路由、系统报告的
格式和详细诊断统一在「设置」查看。

日常使用建议保留 **稳定优先 + 150 ms 初始网络缓冲**。网络缓冲会按抖动调整；
均衡、响应模式仍属于试验选项。调高 EQ 前可适当降低预放大，避免叠加后削波。

## 使用前了解这些边界

- 系统报告的采样率、位深或编码能力不等于耳机实际链路上限；应用不能证明蓝牙编码器或 DAC 最终工作格式，也不会凭空提升音源质量。
- 未压缩 PCM 会占用局域网带宽。发送端默认约 20 ms 后补发原样备份，流量约翻倍，用于抵抗短时丢包，不保证任意网络中零闪断。
- 后台共播依赖系统共享输出及其他应用行为；通话、独占音频和厂商省电策略仍可能影响播放。[后台说明](docs/ANDROID-BACKGROUND.md) 列出具体边界。
- 当前发现与音频协议没有配对认证或加密，请在可信局域网使用。关闭 Windows 的「允许接收端发起连接」可禁用远端启动发送。
- 不承诺固定延迟、CPU 或内存占用，正式发布也不将无声自动测试等同于实体耳机听音。

## 常见问题

**找不到设备？**

确认双方已打开 LanMix，并检查本地网络权限、防火墙与 AP 隔离。先尝试手动 IP；
排查时暂时停用改变局域网路由的 VPN。详见 [界面与连接说明](docs/UI-DISCOVERY.md)。

**听不到声音或偶尔闪断？**

确认发送端正在采集正确的系统输出、接收端已开始接收、各路没有静音。
先恢复稳定优先和 150 ms 缓冲，再复制诊断信息并记录发生时间。
缺粮计数为零也可能存在来源时间线缺口，见 [闪断排查](docs/PLAYBACK-DROPOUTS.md)。

**锁屏或切走页面后停止？**

保留播放服务通知，并检查系统是否限制 LanMix 后台运行。应用已使用前台服务，
但无法绕过厂商进程清理或用户手动强行停止。

**Android 旧版打不开或提示安装冲突？**

使用最新正式 APK；已保留 Android 17 的窗口初始化崩溃修复。
旧调试版与正式版签名不同，需要卸载旧版。请附安卓版本和崩溃文字反馈。

## 开发与构建

项目采用 Rust Workspace，共享协议、DSP、网络、发现和 Android JNI 核心。
Android 外层是 Kotlin / AudioTrack；桌面接收端是 Rust / egui。

```text
crates/
  lanmix-proto       PCM 协议
  lanmix-core        混音、EQ、重采样与时钟漂移处理
  lanmix-network     收包、重排、诊断
  lanmix-audio       桌面音频设备
  lanmix-discovery   局域网发现与连接
  lanmix-sender      Windows 原生窗口 / CLI 发送端
  lanmix-receiver    桌面混音接收端
  lanmix-android     Android JNI
android/            Android 原生 App
vendor/cpal/        保留许可的 WASAPI 采集补丁
```

构建环境、检查命令与发布签名说明见 [源码构建](docs/BUILDING.md)。
[桌面开发构建](docs/DESKTOP-BUILDS.md) 另说明 macOS 云构建和 iOS 现状。
反馈时建议附系统版本、连接方式、播放模式及复制出的诊断信息。

## 开源许可与致谢

LanMix 原创代码采用 **[MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE)**
双许可证，可任选一种；第三方依赖与字体保留各自授权。

感谢 CPAL、egui / eframe、rtrb、jni-rs、Serde、Kotlin 等开源项目。
Android 的视觉设计参考了 [KernelSU Manager](https://github.com/tiann/KernelSU)，
使用原生 Views / Canvas / AGSL 独立实现，没有复制其源码或引入 Miuix。

完整说明见 [第三方声明](THIRD_PARTY_NOTICES.md)、[许可全文](THIRD_PARTY_LICENSES.txt)
及 [依赖清单](third-party/rust-dependencies.json)。重新分发时请保留相关授权与版权声明。
