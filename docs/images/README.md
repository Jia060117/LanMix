# 界面素材

本目录的 PNG 来自 v1.0.3 的实际 Android Views / Canvas 界面代码，使用 Robolectric
API 35 与原生 Skia 离线绘制。它们是界面预览，不是真机截图或播放验收记录。

- `mixer-light.png` / `mixer-dark.png`：浅色、深色混音页的待命状态。
- `equalizer-dark.png`：两个演示音源，选中「笔记本」并使用「人声」预设。
- 设备名与局域网地址是确定性的演示数据。软件 Canvas 使用玻璃底栏的透明回退，
  不能展示实机 GPU 的完整折射效果。

生成方式：设置 `LANMIX_UI_PREVIEW_DIR` 为输出目录，在 Android 构建环境中运行
`testDebugUnitTest --tests io.lanmix.receiver.UiPreviewTest`；测试位于
`android/app/src/test/java/io/lanmix/receiver/UiPreviewTest.kt`。
将输出的 `mixer-notnight.png`、`mixer-night.png`、`equalizer-night.png`
分别复制为本目录对应文件。不修改布局、叠加装饰或伪造播放数据。
