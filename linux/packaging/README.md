# Linux AppImage 打包

首个真机测试包使用 AppImage。构建机要求 x86_64 Ubuntu 24.04 或相同的 glibc 2.39、PipeWire 1.0 基线，并安装 Rust 1.95、PipeWire 开发文件、libclang、X11/Wayland 开发文件、`linuxdeploy` 和 `appimagetool`。

构建命令：

```bash
cd linux
./scripts/build-appimage.sh
```

脚本不会删除或覆盖已有产物。重复构建同一 commit 时，应设置一个新的输出目录：

```bash
MOQCAST_PACKAGE_DIR=/tmp/moqcast-package-2 ./scripts/build-appimage.sh
```

产物位于 `linux/target/package/`，包含：

- `MoQCast-<version>-<commit>-x86_64.AppImage`
- 对应的 SHA-256 文件
- AppDir 内的 `build-info.txt` 和 `linked-libraries.txt`

AppImage 包含应用和普通动态库，但不包含桌面会话服务、portal backend、PipeWire daemon 或 GPU 驱动。真机仍需安装并运行：

界面内置 Noto Sans SC Regular 作为中文 fallback，字体使用 SIL Open Font License 1.1；许可证安装在 AppImage 的 `usr/share/licenses/moqcast/`。

- `xdg-desktop-portal`
- 当前桌面环境对应的 portal backend
- PipeWire
- 可用的 Vulkan 或 OpenGL 图形驱动

第一次验收优先使用 GNOME Wayland 的 Ubuntu 24.04。运行时先检查 `--version`，再启动 UI 完成 mDNS、连接、系统选屏和 Android 播放测试。
