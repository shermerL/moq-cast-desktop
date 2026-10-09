# MoQCast

[English](README.md) | 简体中文

[![Windows CI](https://github.com/shermerL/moq-cast-desktop/actions/workflows/windows.yml/badge.svg?branch=main)](https://github.com/shermerL/moq-cast-desktop/actions/workflows/windows.yml?query=branch%3Amain)
[![macOS CI](https://github.com/shermerL/moq-cast-desktop/actions/workflows/macos.yml/badge.svg?branch=main)](https://github.com/shermerL/moq-cast-desktop/actions/workflows/macos.yml?query=branch%3Amain)
[![Linux CI](https://github.com/shermerL/moq-cast-desktop/actions/workflows/linux-appimage.yml/badge.svg?branch=main)](https://github.com/shermerL/moq-cast-desktop/actions/workflows/linux-appimage.yml?query=branch%3Amain)

**发现附近设备，共享屏幕与窗口，实时观看画面与声音。**

MoQCast 是基于 [Media over QUIC](https://moq.dev) 的开源投屏应用，提供 Windows、macOS 和 Linux 原生桌面端。设备通过局域网发现并直接连接，同一路共享可由多个设备同时观看。

[官网](https://moqtcast.com) · [下载最新版本](https://github.com/shermerL/moq-cast-desktop/releases/latest) · [更新日志](CHANGELOG.md) · [反馈问题](https://github.com/shermerL/moq-cast-desktop/issues)

## 核心特点

- **附近设备发现**：通过 mDNS 查找同一局域网中的设备，无需手动输入地址。
- **屏幕与窗口共享**：使用系统原生采集能力，选择需要共享的内容。
- **一发多看**：多个接收端可以同时观看同一路共享。
- **系统声音共享**：在支持的平台和来源上，将系统声音与画面一起发送。
- **原生播放器**：支持音量调节、静音、全屏，以及视频和音频信息查看。
- **局域网直连**：当前桌面使用流程无需自行部署 Relay。
- **诊断日志**：可导出日志，便于反馈连接、采集和播放问题。

## 平台支持

| 平台 | 主要能力 | 安装包 |
| --- | --- | --- |
| Windows | 显示器与窗口共享、鼠标捕获、系统声音、远端播放 | x64 EXE |
| macOS | 系统选择器中的屏幕与窗口共享、支持来源的系统声音、远端播放 | Universal 2，支持 Apple Silicon 与 Intel |
| Linux | X11 显示器选择、Wayland 屏幕与窗口共享、系统声音、远端播放 | x86-64 AppImage |

声音与选源能力因平台而异：

- Windows 共享窗口时，声音仍来自默认输出设备的系统声音，不是该窗口专属的声音。
- macOS 当前只为应用允许的主显示器来源开启系统声音共享。
- Linux 系统声音通过 PipeWire 采集；X11 当前提供显示器选择，Wayland 的屏幕与窗口由系统 Portal 选择器授权。

## 下载与安装

当前正式版本为 [v0.6.0](https://github.com/shermerL/moq-cast-desktop/releases/tag/v0.6.0)。发行页面为每个安装包提供对应的 SHA-256 校验文件。

### Windows

下载 [Windows x64 EXE](https://github.com/shermerL/moq-cast-desktop/releases/download/v0.6.0/moqcast-windows-v0.6.0.exe) 并运行。

屏幕与窗口采集要求 Windows 10 2004 或更新版本，以及支持采集的图形环境。首次运行时，请允许应用通过防火墙访问所在局域网。

### macOS

下载 [macOS Universal 2 ZIP](https://github.com/shermerL/moq-cast-desktop/releases/download/v0.6.0/MoQCast-macOS-0.6.0.zip)，解压后打开应用。

要求 macOS 14.2 或更新版本；同一安装包支持 Apple Silicon 与 Intel。按系统提示授予屏幕录制和相关音频权限。

当前应用使用 ad hoc 签名，未经过 Apple 公证，首次打开可能需要在系统设置中手动允许。更新应用后，系统也可能要求重新授予采集权限。

### Linux

在[发行页面](https://github.com/shermerL/moq-cast-desktop/releases/tag/v0.6.0)选择与系统匹配的 x86-64 AppImage：Ubuntu 22.04、Ubuntu 24.04、Debian 12 或 Debian 13。

赋予执行权限后运行：

```bash
chmod +x "文件名.AppImage"
./文件名.AppImage
```

Wayland 共享需要桌面环境提供可用的 ScreenCast Portal 与 PipeWire；系统声音共享需要 PipeWire 音频服务。更多系统依赖见 [Linux 说明](linux/README.md)。

## 快速开始

1. 在发布端和接收端打开 MoQCast，并连接到同一局域网。
2. 开启附近设备功能，确认设备能够被发现并建立连接。
3. 在发布端进入屏幕共享，选择屏幕或窗口，并按需开启系统声音。
4. 开始共享后，在接收端选择发布设备并点击观看。
5. 其他接收端可以按相同步骤加入，观看同一路共享。

每个接收端独立控制观看音量和静音。切换共享来源前，请先停止当前共享。

## 工作原理

MoQCast 使用 mDNS 发现附近设备，通过 QUIC 与 MoQ 建立媒体发布和订阅连接，并使用原生媒体能力完成采集、编码、解码与播放。

```text
发布端
屏幕／窗口 → 采集与编码 → MoQ
                          ├──→ 接收端 A：解码与播放
                          └──→ 接收端 B：解码与播放
```

底层协议与媒体能力基于 [moq-dev/moq](https://github.com/moq-dev/moq)。协议和库的说明见 [MoQ 文档](https://moq.dev)。

## 常见问题

### 可以通过互联网远程投屏吗？

当前桌面端面向局域网发现与直连。通过 Relay 进行远程连接属于后续规划。

### 系统声音包含麦克风吗？

系统声音共享采集系统输出，不等同于麦克风输入；可用范围取决于平台与所选来源。

### 不同版本可以互相连接吗？

建议所有设备使用同一版本。底层协议与媒体接口仍在演进，不保证任意历史版本之间互通。

## 开发

桌面应用使用 Rust `1.95.0`。各平台的系统依赖与构建步骤见对应说明：

| 目录 | 内容 |
| --- | --- |
| [windows/](windows/README.md) | Windows 原生应用 |
| [mac/](mac/README.md) | macOS 原生应用 |
| [linux/](linux/README.md) | Linux 原生应用 |
| [shared/](shared/) | 三端共享的界面和基础模块 |

```bash
git clone https://github.com/shermerL/moq-cast-desktop.git
cd moq-cast-desktop
```

从仓库根目录进入对应平台目录，选择一组命令运行。

Windows（PowerShell）：

```powershell
cd windows
cargo run --locked --release
```

macOS：

```bash
cd mac
cargo run --locked
```

Linux：

```bash
cd linux
cargo run --locked --release
```

`main` 是稳定与发布线，使用固定的上游 MoQ `main` 基线；`dev` 用于基于上游 MoQ `dev` 的开发。

## 贡献与反馈

参与开发前请阅读 [贡献指南](CONTRIBUTING.md)。

反馈问题时，请提供应用版本、操作系统和设备信息、发布端与接收端的平台、复现步骤、预期与实际结果，以及导出的诊断日志。界面问题可以附上截图；上传前请检查日志与截图中是否包含个人信息。

## 许可证

本项目可按 [Apache License 2.0](LICENSE-APACHE) 或 [MIT License](LICENSE-MIT) 任选其一使用。

## 致谢

感谢 Luke Curley、[moq-dev/moq](https://github.com/moq-dev/moq) 社区以及所有贡献者。
