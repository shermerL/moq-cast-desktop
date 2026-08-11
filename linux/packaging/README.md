# Linux AppImage 打包

首个真机测试包使用 AppImage。GitHub Actions 从同一个 `main` commit 并行构建以下基线：

| 构建基线 | glibc | 首要验证目标 |
| --- | --- | --- |
| Ubuntu 22.04 | 2.35 | Ubuntu 22.04、Linux Mint 21.x |
| Ubuntu 24.04 | 2.39 | Ubuntu 24.04/26.04、Linux Mint 22.x |
| Debian 12 | 2.36 | Debian 12 |
| Debian 13 | 2.41 | Debian 13 |

表中的目标是验收范围，不是未经测试的兼容承诺。较老 glibc 基线理论上可以在更新系统加载，但 portal、PipeWire、图形驱动和桌面环境仍须逐项真机验证。不同发行版包来自同一源码，不建立发行版专用代码分支。

本机构建要求 x86_64 Linux，并安装 Rust 1.95、PipeWire 开发文件、libclang、X11/Wayland 开发文件、`linuxdeploy` 和 `appimagetool`。

构建命令：

```bash
cd linux
./scripts/build-appimage.sh
```

脚本不会删除或覆盖已有产物。重复构建同一 commit 时，应设置一个新的输出目录：

```bash
MOQCAST_PACKAGE_DIR=/tmp/moqcast-package-2 ./scripts/build-appimage.sh
```

产物位于 `linux/target/package/`，名称包含实际构建基线：

- `MoQCast-<version>-linux-x86_64-<distribution>-glibc<version>.AppImage`
- 对应的 SHA-256 文件
- AppDir 内的 `build-info.txt` 和 `linked-libraries.txt`。`build-info.txt` 记录发行版、glibc、PipeWire 构建版本和首要验证目标。

AppImage 包含应用和普通动态库，但不包含桌面会话服务、portal backend、PipeWire daemon 或 GPU 驱动。真机仍需安装并运行：

界面内置 Noto Sans SC Regular 作为中文 fallback，字体使用 SIL Open Font License 1.1；许可证安装在 AppImage 的 `usr/share/licenses/moqcast/`。

- `xdg-desktop-portal`
- 当前桌面环境对应的 portal backend
- PipeWire
- 可用的 Vulkan 或 OpenGL 图形驱动

每个基线第一次验收都先检查 `--version`，再启动 UI 完成 mDNS、连接、系统选屏和 Android 播放测试。Ubuntu/Mint 优先覆盖 Cinnamon 与 GNOME Wayland，Debian 覆盖 GNOME Wayland；之后补 KDE Wayland 和 X11。
