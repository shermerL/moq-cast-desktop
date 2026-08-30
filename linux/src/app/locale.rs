//! Localized UI copy for the initial English and Chinese interface.

use super::RemoteAudioPhase;

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

    pub(super) fn remote_audio_status(self, phase: RemoteAudioPhase) -> &'static str {
        match (self, phase) {
            (Self::Chinese, RemoteAudioPhase::Idle | RemoteAudioPhase::NoAudio) => "无音频",
            (Self::Chinese, RemoteAudioPhase::Pending) => "等待音频",
            (Self::Chinese, RemoteAudioPhase::TrackSelected) => "已选择音轨",
            (Self::Chinese, RemoteAudioPhase::PcmDecoded) => "PCM 已解码",
            (Self::Chinese, RemoteAudioPhase::PcmSubmitted) => "PCM 提交调用成功",
            (Self::Chinese, RemoteAudioPhase::Failed) => "音频不可用",
            (Self::English, RemoteAudioPhase::Idle | RemoteAudioPhase::NoAudio) => "NO AUDIO",
            (Self::English, RemoteAudioPhase::Pending) => "AUDIO PENDING",
            (Self::English, RemoteAudioPhase::TrackSelected) => "TRACK SELECTED",
            (Self::English, RemoteAudioPhase::PcmDecoded) => "PCM DECODED",
            (Self::English, RemoteAudioPhase::PcmSubmitted) => "PCM SUBMIT OK",
            (Self::English, RemoteAudioPhase::Failed) => "AUDIO ERROR",
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
            Self::Chinese => "设置应用语言和本地诊断。",
            Self::English => "Configure language and local diagnostics.",
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

    pub(super) fn system_audio(self) -> &'static str {
        match self {
            Self::Chinese => "系统音频",
            Self::English => "System audio",
        }
    }

    pub(super) fn system_audio_hint(self) -> &'static str {
        match self {
            Self::Chinese => "同时共享此设备正在播放的声音。",
            Self::English => "Also share sound playing on this device.",
        }
    }

    pub(super) fn share_local_screen(self) -> &'static str {
        match self {
            Self::Chinese => "共享本机屏幕",
            Self::English => "Share this screen",
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

    pub(super) fn media_keeps_mesh(self) -> &'static str {
        match self {
            Self::Chinese => "停止媒体不会断开 mesh。",
            Self::English => "Stopping media keeps the mesh connected.",
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

    pub(super) fn device_details(self) -> &'static str {
        match self {
            Self::Chinese => "设备详情",
            Self::English => "Device details",
        }
    }

    pub(super) fn select_device(self) -> &'static str {
        match self {
            Self::Chinese => "选择一个在线设备查看状态和可用操作。",
            Self::English => "Select an online device to see its status and available actions.",
        }
    }

    pub(super) fn peer_identifier(self) -> &'static str {
        match self {
            Self::Chinese => "设备标识",
            Self::English => "Peer ID",
        }
    }

    pub(super) fn this_device(self) -> &'static str {
        match self {
            Self::Chinese => "本机",
            Self::English => "This device",
        }
    }

    pub(super) fn lan_session(self) -> &'static str {
        match self {
            Self::Chinese => "LAN 会话",
            Self::English => "LAN session",
        }
    }

    pub(super) fn network_endpoints(self) -> &'static str {
        match self {
            Self::Chinese => "网络地址",
            Self::English => "Network endpoints",
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

    pub(super) fn diagnostics(self) -> &'static str {
        match self {
            Self::Chinese => "诊断日志",
            Self::English => "Diagnostic logs",
        }
    }

    pub(super) fn diagnostics_local_hint(self) -> &'static str {
        match self {
            Self::Chinese => "日志仅保存在本机，由你选择是否导出。",
            Self::English => "Logs stay on this device and are exported only when you choose.",
        }
    }

    pub(super) fn detailed_diagnostics(self) -> &'static str {
        match self {
            Self::Chinese => "详细诊断",
            Self::English => "Detailed diagnostics",
        }
    }

    pub(super) fn detailed_diagnostics_hint(self) -> &'static str {
        match self {
            Self::Chinese => "仅提高允许模块的日志级别；网络与 mDNS 模块仍限制为警告。",
            Self::English => {
                "Raises only approved targets; transport and mDNS remain capped at warnings."
            }
        }
    }

    pub(super) fn show_logs(self) -> &'static str {
        match self {
            Self::Chinese => "显示应用日志",
            Self::English => "Show application logs",
        }
    }

    pub(super) fn log_directory(self) -> &'static str {
        match self {
            Self::Chinese => "日志目录",
            Self::English => "Log directory",
        }
    }

    pub(super) fn file_diagnostics_unavailable(self, reason: &str) -> String {
        match self {
            Self::Chinese => format!("文件诊断不可用：{reason}。应用仍可继续运行。"),
            Self::English => {
                format!("File diagnostics unavailable: {reason}. The application can continue.")
            }
        }
    }

    pub(super) fn dropped_diagnostics(self) -> &'static str {
        match self {
            Self::Chinese => "已丢弃诊断项",
            Self::English => "Dropped diagnostics",
        }
    }

    pub(super) fn open_log_directory(self) -> &'static str {
        match self {
            Self::Chinese => "打开日志目录",
            Self::English => "Open log directory",
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
