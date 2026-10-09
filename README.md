# MoQCast

English | [简体中文](README.zh-CN.md)

[![Windows CI](https://github.com/shermerL/moq-cast-desktop/actions/workflows/windows.yml/badge.svg?branch=main)](https://github.com/shermerL/moq-cast-desktop/actions/workflows/windows.yml?query=branch%3Amain)
[![macOS CI](https://github.com/shermerL/moq-cast-desktop/actions/workflows/macos.yml/badge.svg?branch=main)](https://github.com/shermerL/moq-cast-desktop/actions/workflows/macos.yml?query=branch%3Amain)
[![Linux CI](https://github.com/shermerL/moq-cast-desktop/actions/workflows/linux-appimage.yml/badge.svg?branch=main)](https://github.com/shermerL/moq-cast-desktop/actions/workflows/linux-appimage.yml?query=branch%3Amain)

**Discover nearby devices, share screens and windows, and watch with sound in real time.**

MoQCast is an open-source casting app built on [Media over QUIC](https://moq.dev), with native desktop applications for Windows, macOS, and Linux. Devices discover each other and connect directly over the local network. Multiple devices can watch the same shared stream.

[Website](https://moqtcast.com) · [Download](https://github.com/shermerL/moq-cast-desktop/releases/latest) · [Changelog](CHANGELOG.md) · [Report an issue](https://github.com/shermerL/moq-cast-desktop/issues)

## Features

- **Nearby discovery**: find devices on the same local network through mDNS, without entering addresses manually.
- **Screen and window sharing**: choose what to share using native capture capabilities.
- **Multiple viewers**: let several receivers watch the same shared stream.
- **System audio**: send system sound with video on supported platforms and sources.
- **Native playback**: adjust volume, mute, enter fullscreen, and view video and audio information.
- **Direct local connections**: the current desktop workflow does not require you to deploy a relay.
- **Diagnostic logs**: export logs to help investigate connection, capture, and playback issues.

## Platform support

| Platform | Capabilities | Package |
| --- | --- | --- |
| Windows | Display and window sharing, cursor capture, system audio, remote playback | x64 EXE |
| macOS | Screen and window sharing through the system picker, system audio for supported sources, remote playback | Universal 2 for Apple Silicon and Intel |
| Linux | X11 display selection, Wayland screen and window sharing, system audio, remote playback | x86-64 AppImage |

Audio and source selection differ by platform:

- On Windows, sharing a window still captures system audio from the default output device, rather than audio specific to that window.
- On macOS, system audio sharing is currently enabled only for main-display sources allowed by the application.
- On Linux, system audio uses PipeWire. X11 currently offers display selection; Wayland screens and windows are authorized through the system Portal picker.

## Download and install

The current stable release is [v0.6.0](https://github.com/shermerL/moq-cast-desktop/releases/tag/v0.6.0). Each package has a matching SHA-256 checksum file on the release page.

### Windows

Download and run the [Windows x64 EXE](https://github.com/shermerL/moq-cast-desktop/releases/download/v0.6.0/moqcast-windows-v0.6.0.exe).

Screen and window capture requires Windows 10 version 2004 or later and a supported graphics environment. Allow the app through the firewall for your local network when prompted.

### macOS

Download the [macOS Universal 2 ZIP](https://github.com/shermerL/moq-cast-desktop/releases/download/v0.6.0/MoQCast-macOS-0.6.0.zip), extract it, and open the app.

Requires macOS 14.2 or later. The same package supports Apple Silicon and Intel. Grant screen recording and related audio permissions when prompted.

The app is currently ad hoc signed and is not notarized by Apple. You may need to allow it manually in System Settings on first launch. Updates may also require granting capture permissions again.

### Linux

Choose the x86-64 AppImage matching your system from the [release page](https://github.com/shermerL/moq-cast-desktop/releases/tag/v0.6.0): Ubuntu 22.04, Ubuntu 24.04, Debian 12, or Debian 13.

Make the file executable, then run it:

```bash
chmod +x "filename.AppImage"
./filename.AppImage
```

Wayland sharing requires a working ScreenCast Portal and PipeWire from your desktop environment. System audio sharing requires PipeWire audio services. See the [Linux notes](linux/README.md) for additional system dependencies.

## Quick start

1. Open MoQCast on the sender and receivers, connected to the same local network.
2. Enable nearby devices and confirm that devices are discovered and connected.
3. On the sender, open screen sharing, select a screen or window, and enable system audio if needed.
4. Start sharing, then select the sending device on a receiver and choose to watch.
5. Additional receivers can join in the same way to watch the same stream.

Each receiver controls its own playback volume and mute state. Stop the current share before switching capture sources.

## How it works

MoQCast uses mDNS to discover nearby devices, QUIC and MoQ to publish and subscribe to media, and native media capabilities for capture, encoding, decoding, and playback.

```text
Sender
Screen / window → Capture and encode → MoQ
                                        ├──→ Receiver A: decode and play
                                        └──→ Receiver B: decode and play
```

The protocol and media libraries are provided by [moq-dev/moq](https://github.com/moq-dev/moq). See the [MoQ documentation](https://moq.dev) for details about the protocol and libraries.

## Frequently asked questions

### Can I cast remotely over the internet?

The current desktop workflow focuses on local-network discovery and direct connections. Remote connections through a relay are planned.

### Does system audio include my microphone?

System audio sharing captures system output, not microphone input. Availability depends on the platform and selected source.

### Can different versions connect to each other?

Using the same version on all devices is recommended. The underlying protocol and media interfaces are evolving, so interoperability across arbitrary historical versions is not guaranteed.

## Development

Desktop applications use Rust `1.95.0`. See the platform notes for system dependencies and build instructions:

| Directory | Contents |
| --- | --- |
| [windows/](windows/README.md) | Native Windows application |
| [mac/](mac/README.md) | Native macOS application |
| [linux/](linux/README.md) | Native Linux application |
| [shared/](shared/) | UI and foundational modules shared across desktop platforms |

```bash
git clone https://github.com/shermerL/moq-cast-desktop.git
cd moq-cast-desktop
```

Starting from the repository root, choose the commands for your platform.

Windows (PowerShell):

```powershell
cd windows
cargo run --locked --release
```

macOS:

```bash
cd mac
cargo run --locked
```

Linux:

```bash
cd linux
cargo run --locked --release
```

`main` is the stable and release line, using a fixed upstream MoQ `main` baseline. `dev` is reserved for development based on upstream MoQ `dev`.

## Contributing and feedback

Read the [contribution guide](CONTRIBUTING.md) before contributing.

When reporting an issue, include the app version, operating system and device details, sender and receiver platforms, reproduction steps, expected and actual behavior, and exported diagnostic logs. Screenshots can help explain UI issues. Check logs and screenshots for personal information before uploading them.

## License

Licensed under either [Apache License 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT), at your option.

## Acknowledgements

Thanks to Luke Curley, the [moq-dev/moq](https://github.com/moq-dev/moq) community, and all contributors.
