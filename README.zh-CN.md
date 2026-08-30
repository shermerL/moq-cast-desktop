# MoQCast

[English](README.md) | 简体中文

[![Windows CI](https://github.com/shermerL/moq-cast-desktop/actions/workflows/windows.yml/badge.svg?branch=dev)](https://github.com/shermerL/moq-cast-desktop/actions/workflows/windows.yml?query=branch%3Adev)
[![Linux CI](https://github.com/shermerL/moq-cast-desktop/actions/workflows/linux-appimage.yml/badge.svg?branch=dev)](https://github.com/shermerL/moq-cast-desktop/actions/workflows/linux-appimage.yml?query=branch%3Adev)
[![macOS CI](https://github.com/shermerL/moq-cast-desktop/actions/workflows/macos.yml/badge.svg?branch=dev)](https://github.com/shermerL/moq-cast-desktop/actions/workflows/macos.yml?query=branch%3Adev)
[![Windows Lite CI](https://github.com/shermerL/moq-cast-desktop/actions/workflows/windows-lite.yml/badge.svg?branch=dev)](https://github.com/shermerL/moq-cast-desktop/actions/workflows/windows-lite.yml?query=branch%3Adev)

MoQCast 是基于 Media over QUIC 的跨平台低延迟屏幕共享应用。

> 当前版本是开发预览版。各平台的能力和验证进度并不完全一致，界面、兼容性与安装方式仍可能变化。
>
> CI 徽章只表示自动检查和构建状态，不代表真机、硬件或所有平台环境已经验收。

## 平台

| 平台 | 状态 |
| --- | --- |
| Android | 预览版 |
| Windows | 预览版 |
| Linux | 预览版 |
| macOS | 预览版 |
| Windows Lite | 实验性 |
| Web | 实验性 |

此表描述已经实现的产品范围，不代表所有平台组合均已通过真机验证。

## 核心能力

- 自动发现附近设备并建立局域网直连会话。
- 在支持的原生平台上共享 H.264 屏幕画面。
- 在操作系统和来源应用允许采集时包含系统音频。
- 在支持的平台上观看远端屏幕并播放远端音频。
- 与 [MoQCast Android](https://github.com/shermerL/moq-cast) 互通。

## 开发

桌面应用使用 Rust `1.95.0`。请先阅读 [Linux](linux/README.md)、[Windows](windows/README.md)、[macOS](mac/README.md) 或 [Windows Lite](windows-lite/README.md) 的平台说明。

```bash
# Linux
cd linux
cargo run --locked --release

# macOS Nearby 预览
cd mac
cargo run --locked
```

在 Windows PowerShell 中进入 `windows` 目录后运行 `cargo run --locked --release`。

## 许可证

本项目可按 [Apache License 2.0](LICENSE-APACHE) 或 [MIT License](LICENSE-MIT) 任选其一使用。

## 致谢

MoQCast 基于 [moq-dev/moq](https://github.com/moq-dev/moq) 构建。感谢 Luke Curley 与更广泛的 MoQ contributor 社区。
