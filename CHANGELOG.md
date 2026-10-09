# 更新日志 / Changelog

本文件记录 MoQCast Desktop 的重要版本变更。

This file documents notable changes to MoQCast Desktop.

## 0.6.0 - 2026-10-09

### 中文

- Windows 支持选择屏幕或窗口共享，并捕获鼠标指针。
- Linux 支持 X11 显示器选择和 Wayland 系统选择器中的窗口共享，优化 X11 采集效率。
- 三端播放器新增音量、静音和播放信息面板，简化工具栏。
- 改善播放连续性、音画同步及共享停止时的资源清理。
- 更新媒体与传输依赖，统一构建和诊断日志中的版本来源。

### English

- Select a screen or window to share on Windows, with mouse cursor capture.
- Select displays on X11 and share windows through the Wayland system picker, with more efficient X11 capture.
- Add volume, mute, and playback information controls across desktop platforms, and simplify the player toolbar.
- Improve playback continuity, audio/video synchronization, and cleanup when sharing stops.
- Update media and transport dependencies and unify build provenance in diagnostics.

## 0.5.0-dev.1 - 2026-09-02

### 中文

- 更新桌面界面与应用图标，统一附近设备、屏幕共享、观看和设置页面。
- 改善设备列表点击、列表高度和播放器控制栏布局。
- 改进附近设备发现暂时中断时的屏幕路由状态处理。
- 首次提供 macOS Universal 2 预发布包，支持屏幕共享、主显示器系统音频共享和远端播放。
- 修复 macOS 无音轨视频播放时可能累积延迟的问题。

### English

- Refresh the desktop interface and application icons across Nearby, Screen share, Watch, and Settings.
- Improve device-row interaction, list sizing, and player control layout.
- Improve screen-route handling during transient nearby discovery loss.
- Introduce the macOS Universal 2 preview with screen sharing, main-display system audio sharing, and remote playback.
- Fix accumulating playback delay for video-only streams on macOS.

## 0.4.1-dev.4 - 2026-08-30

### 中文

- 改进 Linux 和 Windows 的附近设备扫描、离线设备清理和本机身份显示

### English

- Improve Nearby scanning, offline device cleanup, and local device identity on Linux and Windows

## 0.4.1-dev.3 - 2026-08-26

### 中文

- 改善 Linux 远端音频播放的音画同步、短时抖动容忍与播放器布局
- 改善 Windows 远端音频在短时网络抖动下的连续性
- Linux 和 Windows 增加本地诊断日志
- Linux 和 Windows 观看端不再转发远端媒体，避免多个播放端相互影响

### English

- Improve Linux remote audio/video synchronization, brief jitter tolerance, and player layout
- Improve Windows remote audio continuity during brief network jitter
- Add local diagnostic logs on Linux and Windows
- Prevent Linux and Windows viewers from forwarding remote media to avoid interference between viewers

## 0.4.1-dev.2 - 2026-08-24

### 中文

- Linux 和 Windows 已迁移至当前 moq-dev dev 基线，统一使用 `moq-tokio`、`moq-audio` 和 `moq-video`
- Linux 兼容较旧的 PipeWire 0.3 头文件

### English

- Migrate Linux and Windows to the current moq-dev dev baseline with `moq-tokio`, `moq-audio`, and `moq-video`
- Support older PipeWire 0.3 headers on Linux

## 0.4.1-dev.1 - 2026-08-23

### 中文

- Linux 和 Windows 支持发现并连接同一局域网内的 MoQCast 设备
- Linux 支持发布屏幕与系统音频，并播放远端屏幕与音频
- Windows 支持发布屏幕与系统音频，并播放远端屏幕与音频
- Windows 支持选择兼容 1080p 或原生 QHD H.264 编码策略
- 改善 Linux 设备与音频选项的选择状态显示

### English

- Discover and connect to MoQCast devices on the same LAN from Linux and Windows
- Publish the screen and system audio, and play remote screen and audio on Linux
- Publish the screen and system audio, and play remote screen and audio on Windows
- Select compatible 1080p or native QHD H.264 encoding policies on Windows
- Improve selected states for Linux device and audio options
