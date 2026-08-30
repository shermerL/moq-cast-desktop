# MoQCast Desktop

MoQCast Desktop 是基于 Media over QUIC 的局域网投屏应用，支持 Linux 和 Windows。macOS 原生端正在建立基础能力，尚未发布可用版本。

MoQCast Desktop is a LAN screen-sharing application for Linux and Windows, built with Media over QUIC. The native macOS app is at the foundation stage and does not have a usable release yet.

> 当前为开发预发布版本。This project is currently a development prerelease.

## 功能 / Features

- 局域网设备发现与 QUIC 直连 / LAN discovery and direct QUIC connections
- 发布屏幕和系统音频 / Publish the screen and system audio
- 播放远端屏幕和音频 / Play a remote screen and audio
- 与 [MoQCast Android](https://github.com/shermerL/moq-cast) 互联 / Interoperate with MoQCast Android

## 下载 / Downloads

从 [GitHub Releases](https://github.com/shermerL/moq-cast-desktop/releases) 下载 Windows x86-64 EXE 或 Linux x86-64 AppImage。

Download the Windows x86-64 EXE or Linux x86-64 AppImage from [GitHub Releases](https://github.com/shermerL/moq-cast-desktop/releases).

## 从源码运行 / Run From Source

项目使用 Rust `1.95.0`。The project uses Rust `1.95.0`.

```bash
# Linux
cd linux
cargo run --locked --release

# Windows PowerShell
cd windows
cargo run --locked --release

# macOS foundation, media is not implemented yet
cd mac
cargo run --locked
```

平台说明见 [Linux](linux/README.md)、[Windows](windows/README.md) 和 [macOS](mac/README.md)。

See the platform notes for [Linux](linux/README.md), [Windows](windows/README.md), and [macOS](mac/README.md).

## MoQTCast Lite for Windows

[`windows-lite/`](windows-lite/README.md) 是独立的轻量 notification-area 应用，只浏览局域网中的 MoQ 设备在线状态，并通过进程期授权的 loopback API 把清洗后 presence 交给 MoQTCast Connect 页面。它不发布服务，不处理或代理媒体。

[`windows-lite/`](windows-lite/README.md) is a separate lightweight notification-area application. It browses MoQ device presence on the LAN and exposes only sanitized presence to the MoQTCast Connect page through a process-authorized loopback API. It does not advertise a service or handle or proxy media.

## License

Licensed under either [Apache License 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT), at your option.

## Acknowledgements

MoQCast Desktop 基于 [moq-dev/moq](https://github.com/moq-dev/moq) 构建。感谢 Luke Curley 及其他 MoQ contributors。

MoQCast Desktop builds on [moq-dev/moq](https://github.com/moq-dev/moq). Thanks to Luke Curley and the other MoQ contributors.
