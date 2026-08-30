# MoQCast

English | [简体中文](README.zh-CN.md)

[![Windows CI](https://github.com/shermerL/moq-cast-desktop/actions/workflows/windows.yml/badge.svg?branch=dev)](https://github.com/shermerL/moq-cast-desktop/actions/workflows/windows.yml?query=branch%3Adev)
[![Linux CI](https://github.com/shermerL/moq-cast-desktop/actions/workflows/linux-appimage.yml/badge.svg?branch=dev)](https://github.com/shermerL/moq-cast-desktop/actions/workflows/linux-appimage.yml?query=branch%3Adev)
[![macOS CI](https://github.com/shermerL/moq-cast-desktop/actions/workflows/macos.yml/badge.svg?branch=dev)](https://github.com/shermerL/moq-cast-desktop/actions/workflows/macos.yml?query=branch%3Adev)
[![Windows Lite CI](https://github.com/shermerL/moq-cast-desktop/actions/workflows/windows-lite.yml/badge.svg?branch=dev)](https://github.com/shermerL/moq-cast-desktop/actions/workflows/windows-lite.yml?query=branch%3Adev)

MoQCast is a cross-platform, low-latency screen-sharing app built with Media over QUIC.

> This is a development preview. Capabilities and validation differ by platform, and the UI, compatibility, and installation flow may change.
>
> CI badges report automated checks and builds only. They do not prove real-device, hardware, or complete platform-environment acceptance.

## Platforms

| Platform | Status |
| --- | --- |
| Android | Preview |
| Windows | Preview |
| Linux | Preview |
| macOS | Preview |
| Windows Lite | Experimental |
| Web | Experimental |

The table describes implemented product scope, not a guarantee that every platform pair has passed real-device validation.

## Core capabilities

- Discover nearby devices and establish direct local-network sessions automatically.
- Share H.264 screen video on supported native platforms.
- Include system audio when the operating system and source allow capture.
- Watch remote screen video and play remote audio on supported platforms.
- Interoperate with [MoQCast Android](https://github.com/shermerL/moq-cast).

## Development

Desktop applications use Rust `1.95.0`. Start with the platform notes for [Linux](linux/README.md), [Windows](windows/README.md), [macOS](mac/README.md), or [Windows Lite](windows-lite/README.md).

```bash
# Linux
cd linux
cargo run --locked --release

# macOS Nearby preview
cd mac
cargo run --locked
```

On Windows, run `cargo run --locked --release` from the `windows` directory in PowerShell.

## License

Licensed under either [Apache License 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT), at your option.

## Acknowledgements

MoQCast builds on [moq-dev/moq](https://github.com/moq-dev/moq). Thanks to Luke Curley and the wider MoQ contributor community.
