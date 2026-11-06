# LanMix 图标

`lanmix.svg` 是项目原创的调音推子标识，配色为深蓝 `#162339` 与浅蓝 `#87C1FF`，
沿用 Android 的视觉语言。仓库 README、Windows 文件 / 窗口 / 托盘，以及 Android
启动图标、主题图标与后台通知使用同一标识。

安装 Pillow 后，在仓库根目录执行 `python scripts/generate-icons.py`，即可从 SVG
生成 PNG、含 16 / 24 / 32 / 48 / 64 / 128 / 256 像素的 ICO 及 Android Vector Drawable。
Android 的自适应图标分为独立背景和安全区域内的前景；单色版本用于系统主题与通知。
这些图形资源采用项目的 MIT OR Apache-2.0 双许可证。
