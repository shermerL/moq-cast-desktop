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

    pub(super) fn settings_description(self) -> &'static str {
        match self {
            Self::Chinese => "语言与应用信息。网络安全设置由运行时管理。",
            Self::English => "Language and app information. Network security is runtime-managed.",
        }
    }

    pub(super) fn nearby_description(self) -> &'static str {
        match self {
            Self::Chinese => "自动连接同一局域网内的 MoQCast 设备。",
            Self::English => "Automatically connect to MoQCast devices on this network.",
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

    pub(super) fn discovery_idle(self) -> &'static str {
        match self {
            Self::Chinese => "扫描已停止",
            Self::English => "Scan stopped",
        }
    }

    pub(super) fn discovery_ready(self) -> &'static str {
        match self {
            Self::Chinese => "发现服务运行中",
            Self::English => "Discovery active",
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

    pub(super) fn no_devices_hint(self) -> &'static str {
        match self {
            Self::Chinese => "请确保另一台设备已开启局域网模式并连接同一网络。",
            Self::English => "Make sure another device has LAN mode enabled on the same network.",
        }
    }

    pub(super) fn watch(self) -> &'static str {
        match self {
            Self::Chinese => "观看",
            Self::English => "Watch",
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

    pub(super) fn transport_waiting(self) -> &'static str {
        match self {
            Self::Chinese => "等待 mesh 连接",
            Self::English => "Waiting for mesh",
        }
    }

    pub(super) fn transport_inbound_role(self) -> &'static str {
        match self {
            Self::Chinese => "由对端发起连接",
            Self::English => "Remote peer dials this device",
        }
    }

    pub(super) fn transport_connecting(self) -> &'static str {
        match self {
            Self::Chinese => "正在建立 mesh…",
            Self::English => "Connecting mesh...",
        }
    }

    pub(super) fn transport_connected(self) -> &'static str {
        match self {
            Self::Chinese => "Mesh 已连接",
            Self::English => "Mesh connected",
        }
    }

    pub(super) fn transport_failed(self) -> &'static str {
        match self {
            Self::Chinese => "Mesh 连接失败",
            Self::English => "Mesh connection failed",
        }
    }

    pub(super) fn screen_available(self) -> &'static str {
        match self {
            Self::Chinese => "屏幕可观看",
            Self::English => "Screen available",
        }
    }

    pub(super) fn screen_unavailable(self) -> &'static str {
        match self {
            Self::Chinese => "未共享屏幕",
            Self::English => "No shared screen",
        }
    }

    pub(super) fn inbound_sessions(self) -> &'static str {
        match self {
            Self::Chinese => "未归属入站连接",
            Self::English => "Unattributed inbound sessions",
        }
    }

    pub(super) fn outbound_sessions(self) -> &'static str {
        match self {
            Self::Chinese => "出站连接",
            Self::English => "Outbound sessions",
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
            Self::Chinese => "共享本机屏幕，或观看一个已发布的远端屏幕。",
            Self::English => "Share this desktop or view one available remote screen.",
        }
    }

    pub(super) fn not_connected(self) -> &'static str {
        match self {
            Self::Chinese => "尚未连接设备",
            Self::English => "No device connected",
        }
    }

    pub(super) fn connect_first(self) -> &'static str {
        match self {
            Self::Chinese => "正在等待同一局域网内的设备建立 mesh。",
            Self::English => "Waiting for a device on this network to join the mesh.",
        }
    }

    pub(super) fn choose_screen(self) -> &'static str {
        match self {
            Self::Chinese => "选择屏幕",
            Self::English => "Choose screen",
        }
    }

    pub(super) fn preparing_share(self) -> &'static str {
        match self {
            Self::Chinese => "正在等待系统选择屏幕…",
            Self::English => "Waiting for the system screen picker...",
        }
    }

    pub(super) fn sharing_screen(self) -> &'static str {
        match self {
            Self::Chinese => "正在共享屏幕",
            Self::English => "Sharing your screen",
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

    pub(super) fn retry(self) -> &'static str {
        match self {
            Self::Chinese => "重试",
            Self::English => "Retry",
        }
    }

    pub(super) fn devices(self) -> &'static str {
        match self {
            Self::Chinese => "设备",
            Self::English => "Devices",
        }
    }

    pub(super) fn mesh_status_hint(self) -> &'static str {
        match self {
            Self::Chinese => "设备行只显示精确出站状态；入站连接无法安全归属到具体设备。",
            Self::English => {
                "Device rows show exact outbound state. Inbound sessions cannot yet be assigned safely."
            }
        }
    }

    pub(super) fn media_idle(self) -> &'static str {
        match self {
            Self::Chinese => "屏幕媒体空闲",
            Self::English => "Screen media is idle",
        }
    }

    pub(super) fn media_idle_hint(self) -> &'static str {
        match self {
            Self::Chinese => "选择一个本机屏幕进行共享，或从附近设备观看可用屏幕。",
            Self::English => {
                "Choose a local display to share, or watch an available nearby screen."
            }
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

    pub(super) fn about_description(self) -> &'static str {
        match self {
            Self::Chinese => "基于 mDNS 与 MoQ / QUIC 的局域网 screen-only mesh。",
            Self::English => "A screen-only LAN mesh built on mDNS and MoQ / QUIC.",
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
