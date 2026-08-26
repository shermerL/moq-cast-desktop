# MoQCast Desktop

MoQCast Desktop 是基于 Media over QUIC 的局域网投屏应用，支持 Linux 和 Windows。

MoQCast Desktop is a LAN screen-sharing application for Linux and Windows, built with Media over QUIC.

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
```

平台说明见 [Linux](linux/README.md) 和 [Windows](windows/README.md)。

See the platform notes for [Linux](linux/README.md) and [Windows](windows/README.md).

## License

Licensed under either [Apache License 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT), at your option.

## Acknowledgements

MoQCast Desktop 基于 [moq-dev/moq](https://github.com/moq-dev/moq) 构建。感谢 Luke Curley 及其他 MoQ contributors。

MoQCast Desktop builds on [moq-dev/moq](https://github.com/moq-dev/moq). Thanks to Luke Curley and the other MoQ contributors.
