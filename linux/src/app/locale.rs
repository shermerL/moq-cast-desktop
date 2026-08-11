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

    pub(super) fn desktop(self) -> &'static str {
        match self {
            Self::Chinese => "桌面端",
            Self::English => "Desktop",
        }
    }

    pub(super) fn nearby(self) -> &'static str {
        match self {
            Self::Chinese => "附近设备",
            Self::English => "Nearby",
        }
    }

    pub(super) fn screen_share(self) -> &'static str {
        match self {
            Self::Chinese => "屏幕共享",
            Self::English => "Screen share",
        }
    }

    pub(super) fn settings(self) -> &'static str {
        match self {
            Self::Chinese => "设置",
            Self::English => "Settings",
        }
    }

    pub(super) fn nearby_description(self) -> &'static str {
        match self {
            Self::Chinese => "发现并连接同一局域网内的 MoQCast 设备。",
            Self::English => "Find and connect to MoQCast devices on this network.",
        }
    }

    pub(super) fn start_scan(self) -> &'static str {
        match self {
            Self::Chinese => "开始扫描",
            Self::English => "Start scan",
        }
    }

    pub(super) fn stop_scan(self) -> &'static str {
        match self {
            Self::Chinese => "停止扫描",
            Self::English => "Stop scan",
        }
    }

    pub(super) fn scanning(self) -> &'static str {
        match self {
            Self::Chinese => "正在扫描局域网…",
            Self::English => "Scanning the local network...",
        }
    }

    pub(super) fn no_devices(self) -> &'static str {
        match self {
            Self::Chinese => "暂未发现设备",
            Self::English => "No devices found",
        }
    }

    pub(super) fn no_devices_hint(self) -> &'static str {
        match self {
            Self::Chinese => "请确保另一台设备已开启局域网模式并连接同一网络。",
            Self::English => "Make sure another device has LAN mode enabled on the same network.",
        }
    }

    pub(super) fn connect(self) -> &'static str {
        match self {
            Self::Chinese => "连接并共享",
            Self::English => "Connect and share",
        }
    }

    pub(super) fn connecting(self) -> &'static str {
        match self {
            Self::Chinese => "正在连接…",
            Self::English => "Connecting...",
        }
    }

    pub(super) fn disconnect(self) -> &'static str {
        match self {
            Self::Chinese => "断开连接",
            Self::English => "Disconnect",
        }
    }

    pub(super) fn fingerprint_pinning(self) -> &'static str {
        match self {
            Self::Chinese => "TLS 使用广播指纹固定",
            Self::English => "TLS pinned to the advertised fingerprint",
        }
    }

    pub(super) fn share_description(self) -> &'static str {
        match self {
            Self::Chinese => "通过系统选择器共享一个屏幕。",
            Self::English => "Share one display through the system picker.",
        }
    }

    pub(super) fn not_connected(self) -> &'static str {
        match self {
            Self::Chinese => "尚未连接设备",
            Self::English => "No device connected",
        }
    }

    pub(super) fn connected(self) -> &'static str {
        match self {
            Self::Chinese => "已连接设备",
            Self::English => "Connected device",
        }
    }

    pub(super) fn connect_first(self) -> &'static str {
        match self {
            Self::Chinese => "请先在附近设备页面连接接收端。",
            Self::English => "Connect a receiver from Nearby first.",
        }
    }

    pub(super) fn choose_screen(self) -> &'static str {
        match self {
            Self::Chinese => "选择屏幕",
            Self::English => "Choose screen",
        }
    }

    pub(super) fn stop_sharing(self) -> &'static str {
        match self {
            Self::Chinese => "停止共享",
            Self::English => "Stop sharing",
        }
    }

    pub(super) fn language(self) -> &'static str {
        match self {
            Self::Chinese => "语言",
            Self::English => "Language",
        }
    }

    pub(super) fn about(self) -> &'static str {
        match self {
            Self::Chinese => "关于",
            Self::English => "About",
        }
    }

    pub(super) fn app_version(self) -> &'static str {
        match self {
            Self::Chinese => "应用版本",
            Self::English => "App version",
        }
    }

    pub(super) fn protocol(self) -> &'static str {
        match self {
            Self::Chinese => "传输协议",
            Self::English => "Transport",
        }
    }
}
