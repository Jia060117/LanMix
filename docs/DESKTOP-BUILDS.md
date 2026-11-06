# 桌面构建与下载

v1.0.0 起 Windows 与 Android 统一正式版本。正式下载只提供这两个平台，
入口见 [Releases](https://github.com/Jia060117/LanMix/releases/latest)。下文的
Actions 产物属于开发构建，包含尚未完成实体音频验收的 Linux / macOS。
开发构建日志记录 Git 提交号，其他平台的归档名也带提交号；音频协议保持兼容。

## 获取程序

GitHub 仓库的 **Actions → Rust desktop checks → 成功的运行 → Artifacts** 中提供：

| 构建 | 包格式 | 内容与边界 |
|---|---|---|
| Windows x64 | 两个 EXE，分别提供 | `LanMix-sender-windows-x64` 为发送端；`LanMix-receiver-windows-x64` 为混音接收端 |
| Linux x64 | tar.gz | 命令行发送端、图形混音接收端；在 Ubuntu 22.04 构建 |
| macOS Apple Silicon | ZIP | ARM64 的 `LanMix.app` 混音接收端、命令行程序 |
| macOS Intel | ZIP | x64 的 `LanMix.app` 混音接收端、命令行程序 |

Artifacts 保留 30 天，通常需登录 GitHub 下载；这不是 GitHub Release，也不会创建
发布说明。Windows 每个 Artifact 仅上传对应 EXE；GitHub 的 Artifact 下载可能自动套一层 ZIP，
正式 Release 则直接上传两个 EXE 供分别下载。版本、提交和摘要见构建日志。
Linux / macOS 开发包保留归档、`SHA256SUMS.txt` 和包内 `BUILD-INFO.json`。
推送与手动运行都会在检查成功后上传，PR 只检查和准备文件，不上传。

Windows 直接双击 `lanmix-sender.exe` 发送声音；`lanmix-receiver.exe` 用于电脑接收、混音。
发送端同时保留 CLI，不另附重复的 `lanmix.exe`。两个 EXE 都内置压缩的项目与第三方许可，
可通过 `--licenses 文件路径` 导出，无需额外携带许可目录。
Linux 解压后在图形会话中运行 `./lanmix-receiver`；命令行程序用 `./lanmix-sender`。
macOS 按芯片下载，解压后打开 `LanMix.app`；包内同名命令行接收端可用于无声诊断。
Mac App 使用本地 ad-hoc 签名，没有冒用任何发布者，也未做 Developer ID 公证；
下载后系统可能要求在“系统设置 → 隐私与安全性”中确认打开。

Linux 需要 ALSA、udev、OpenGL 及 X11/Wayland 运行库。Ubuntu 22.04 可以安装：

```sh
sudo apt-get install libasound2 libudev1 libxkbcommon0 libwayland-client0 libegl1 libgl1 libxcursor1 libxrandr2 libxi6
```

Linux 与 macOS 的发送端目前使用命令行。先运行 `./lanmix-sender devices`，
再通过 `./lanmix-sender send --input 输入编号 --to 手机IP:46210` 选择已配置的音频输入。
Linux 系统声音需要 monitor 输入；macOS 自动系统声音采集后端尚未实现，
不能将编译成功理解为已经支持直接推送整个系统的声音。不会自动选择麦克风。

## 没有 Mac 能否编译

可以使用 GitHub 托管的 macOS 构建机，无需本机安装 macOS。工作流使用独立的
Apple Silicon 和 Intel 构建机，运行格式检查、Clippy、Rust 测试、真实 UDP
无声混音检查，再生成对应下载包。无声检查不会验证实体声卡、蓝牙、窗口操作
或最低系统版本；需要目标设备补充验收。

iOS/iPadOS 目前没有 App 工程。共享 Rust 核心可以复用，但仍需补充 SwiftUI
界面、Rust FFI、iOS 音频输出、后台音频和本地网络权限，再用 Mac 上的 Xcode/SDK
构建。云端 Mac 也可以完成这些构建；真机安装与分发还需相应的签名和设备配置。
仅安装 Rust 的 iOS target 不会生成一个可安装的 IPA。现阶段没有 iOS 安装包。

## 本机打包

在对应系统、对应架构编译，使用 Python 3.11+：

```sh
cargo build --locked --release -p lanmix-sender -p lanmix-receiver
python scripts/smoke_test.py --bin-dir target/release
python scripts/package-desktop.py --platform linux-x64
```

将平台改为 `windows-x64`、`macos-arm64` 或 `macos-x64`。Windows 默认输出目录为
`dist/windows-x64-standalone/`，其中只有两个 EXE；其他平台沿用归档格式。脚本校验宿主平台与
二进制版本，不接受在 Windows 上给 Windows 程序贴上 Mac 标签。Mac 构建另外
需要 Xcode Command Line Tools，工作流将部署目标设为 macOS 11.0；这个声明不
代替在旧版系统上的实机运行测试。生成物位于被 Git 忽略的 `dist/`。
