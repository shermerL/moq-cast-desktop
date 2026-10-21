//! Localized UI copy for the initial English and Chinese interface.

/// A supported interface language.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Locale {
    /// Simplified Chinese.
    #[default]
    Chinese,
    /// English.
    English,
}

impl Locale {
    pub(super) fn text<'a>(self, zh: &'a str, en: &'a str) -> &'a str {
        match self {
            Self::Chinese => zh,
            Self::English => en,
        }
    }

    /// Parse a persisted locale identifier.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "zh-CN" => Some(Self::Chinese),
            "en" => Some(Self::English),
            _ => None,
        }
    }

    /// Return the persisted locale identifier.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chinese => "zh-CN",
            Self::English => "en",
        }
    }

    pub(super) fn nearby(self) -> &'static str {
        match self {
            Self::Chinese => "附近设备",
            Self::English => "Nearby",
        }
    }

    pub(super) fn watch(self) -> &'static str {
        match self {
            Self::Chinese => "观看",
            Self::English => "Watch",
        }
    }

    pub(super) fn settings(self) -> &'static str {
        match self {
            Self::Chinese => "设置",
            Self::English => "Settings",
        }
    }

    pub(super) fn discovery_error(self) -> &'static str {
        match self {
            Self::Chinese => "发现服务需要恢复",
            Self::English => "Discovery needs attention",
        }
    }

    pub(super) fn no_devices(self) -> &'static str {
        match self {
            Self::Chinese => "暂未发现设备",
            Self::English => "No devices found",
        }
    }

    pub(super) fn discovery_found(self) -> &'static str {
        match self {
            Self::Chinese => "已发现",
            Self::English => "Discovered",
        }
    }

    pub(super) fn discovery_lost(self) -> &'static str {
        match self {
            Self::Chinese => "发现已丢失",
            Self::English => "Discovery lost",
        }
    }

    pub(super) fn enable_nearby_before_sharing(self) -> &'static str {
        match self {
            Self::Chinese => "开启附近设备后即可共享。",
            Self::English => "Enable nearby devices before sharing.",
        }
    }

    pub(super) fn system_audio(self) -> &'static str {
        match self {
            Self::Chinese => "系统音频",
            Self::English => "System audio",
        }
    }

    pub(super) fn system_audio_hint(self) -> &'static str {
        match self {
            Self::Chinese => "共享此设备的系统声音，不仅是选中窗口的声音。",
            Self::English => "Share system sound, not just sound from the selected window.",
        }
    }

    pub(super) fn preparing_share(self) -> &'static str {
        match self {
            Self::Chinese => "正在准备共享画面…",
            Self::English => "Preparing the selected source...",
        }
    }

    pub(super) fn sharing_screen(self) -> &'static str {
        match self {
            Self::Chinese => "正在共享画面",
            Self::English => "Sharing the selected source",
        }
    }

    pub(super) fn stopping_share(self) -> &'static str {
        match self {
            Self::Chinese => "正在停止共享…",
            Self::English => "Stopping screen share...",
        }
    }

    pub(super) fn stop_sharing(self) -> &'static str {
        match self {
            Self::Chinese => "停止共享",
            Self::English => "Stop sharing",
        }
    }

    pub(super) fn preparing_view(self) -> &'static str {
        match self {
            Self::Chinese => "正在打开远端屏幕…",
            Self::English => "Opening remote screen...",
        }
    }

    pub(super) fn stop_watching(self) -> &'static str {
        match self {
            Self::Chinese => "停止观看",
            Self::English => "Stop watching",
        }
    }

    pub(super) fn mute(self) -> &'static str {
        match self {
            Self::Chinese => "静音",
            Self::English => "Mute",
        }
    }

    pub(super) fn unmute(self) -> &'static str {
        match self {
            Self::Chinese => "取消静音",
            Self::English => "Unmute",
        }
    }

    pub(super) fn playback_volume(self) -> &'static str {
        match self {
            Self::Chinese => "播放音量",
            Self::English => "Playback volume",
        }
    }

    pub(super) fn playback_audio_unavailable(self) -> &'static str {
        match self {
            Self::Chinese => "当前屏幕没有可播放音频",
            Self::English => "No playable audio is available for this screen",
        }
    }

    pub(super) fn open_nearby(self) -> &'static str {
        match self {
            Self::Chinese => "打开附近设备",
            Self::English => "Open Nearby",
        }
    }

    pub(super) fn nearby_device(self) -> &'static str {
        match self {
            Self::Chinese => "附近设备",
            Self::English => "Nearby device",
        }
    }

    pub(super) fn viewing_screen(self) -> &'static str {
        match self {
            Self::Chinese => "正在观看远端屏幕",
            Self::English => "Watching a remote screen",
        }
    }

    pub(super) fn stopping_view(self) -> &'static str {
        match self {
            Self::Chinese => "正在停止观看…",
            Self::English => "Stopping remote screen...",
        }
    }

    pub(super) fn enter_fullscreen(self) -> &'static str {
        match self {
            Self::Chinese => "进入全屏",
            Self::English => "Fullscreen",
        }
    }

    pub(super) fn exit_fullscreen(self) -> &'static str {
        match self {
            Self::Chinese => "退出全屏",
            Self::English => "Exit fullscreen",
        }
    }

    pub(super) fn waiting_for_first_frame(self) -> &'static str {
        match self {
            Self::Chinese => "等待首帧",
            Self::English => "Waiting for first frame",
        }
    }

    pub(super) fn operation_failed(self) -> &'static str {
        match self {
            Self::Chinese => "操作未完成。请重试；详细信息已写入本地诊断日志。",
            Self::English => {
                "The operation did not complete. Try again; details are in local diagnostic logs."
            }
        }
    }

    pub(super) fn device_details(self) -> &'static str {
        match self {
            Self::Chinese => "设备详情",
            Self::English => "Device details",
        }
    }

    pub(super) fn media_idle(self) -> &'static str {
        match self {
            Self::Chinese => "屏幕媒体空闲",
            Self::English => "Screen media is idle",
        }
    }

    pub(super) fn language(self) -> &'static str {
        match self {
            Self::Chinese => "语言",
            Self::English => "Language",
        }
    }

    pub(super) fn developer_mode(self) -> &'static str {
        match self {
            Self::Chinese => "开发者模式",
            Self::English => "Developer mode",
        }
    }

    pub(super) fn detailed_diagnostics(self) -> &'static str {
        match self {
            Self::Chinese => "详细诊断",
            Self::English => "Detailed diagnostics",
        }
    }

    pub(super) fn show_logs(self) -> &'static str {
        match self {
            Self::Chinese => "显示应用日志",
            Self::English => "Show application logs",
        }
    }

    pub(super) fn file_diagnostics_unavailable_short(self) -> &'static str {
        match self {
            Self::Chinese => "文件诊断不可用",
            Self::English => "File diagnostics unavailable",
        }
    }

    pub(super) fn dropped_diagnostics(self) -> &'static str {
        match self {
            Self::Chinese => "已丢弃诊断项",
            Self::English => "Dropped diagnostics",
        }
    }

    pub(super) fn export_logs(self) -> &'static str {
        match self {
            Self::Chinese => "导出本地日志",
            Self::English => "Export local logs",
        }
    }

    pub(super) fn application_logs(self) -> &'static str {
        match self {
            Self::Chinese => "应用日志",
            Self::English => "Application logs",
        }
    }

    pub(super) fn log_level(self) -> &'static str {
        match self {
            Self::Chinese => "最低级别",
            Self::English => "Minimum level",
        }
    }

    pub(super) fn search_logs(self) -> &'static str {
        match self {
            Self::Chinese => "搜索",
            Self::English => "Search",
        }
    }

    pub(super) fn search_logs_hint(self) -> &'static str {
        match self {
            Self::Chinese => "目标、线程或事件",
            Self::English => "Target, thread, or event",
        }
    }

    pub(super) fn pause_auto_scroll(self) -> &'static str {
        match self {
            Self::Chinese => "暂停自动滚动",
            Self::English => "Pause auto-scroll",
        }
    }

    pub(super) fn copy_visible_logs(self) -> &'static str {
        match self {
            Self::Chinese => "复制当前结果",
            Self::English => "Copy visible logs",
        }
    }

    pub(super) fn no_log_entries(self) -> &'static str {
        match self {
            Self::Chinese => "当前筛选条件下没有日志。",
            Self::English => "No logs match the current filters.",
        }
    }

    pub(super) fn export_completed(self, path: &str) -> String {
        match self {
            Self::Chinese => format!("本地日志已导出到 {path}"),
            Self::English => format!("Local logs exported to {path}"),
        }
    }

    pub(super) fn diagnostics_error(self, error: &str) -> String {
        match self {
            Self::Chinese => format!("本地诊断操作失败：{error}"),
            Self::English => format!("Local diagnostics action failed: {error}"),
        }
    }
}
