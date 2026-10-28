//! 参考名单。Poise 不靠名单决定"做什么"（游戏按行为识别，系统进程一律不碰），这里只用于安全：
//! 确保反作弊、虚拟机、关键系统服务，以及输入、显示、声音、安全软件相关的程序永远不被触碰；
//!
//! 所有名称均为小写。

/// 反作弊程序的进程名。
const ANTI_CHEAT_PROCESSES: &[&str] = &[
    "vgc.exe",
    "vgtray.exe",
    "easyanticheat.exe",
    "easyanticheat_eos.exe",
    "beservice.exe",
    "beservice_x64.exe",
    "sguard64.exe",
    "sguardsvc64.exe",
    "ace-tray.exe",
    "faceitservice.exe",
    "faceit.exe",
    "gameguard.exe",
    "xigncode.exe",
];

/// 反作弊服务名。
const ANTI_CHEAT_SERVICES: &[&str] = &[
    "vgc",
    "vgk",
    "easyanticheat",
    "easyanticheat_eos",
    "beservice",
    "ace-base",
    "anticheatexpert",
    "faceit",
];

/// 虚拟机工作进程。
const VM_PROCESSES: &[&str] = &["vmmem", "vmmemwsl", "vmwp.exe", "vmcompute.exe", "vmms.exe"];

/// 用户会话里不参与调度的程序（名称前缀）：输入法、外设助手、显卡与声卡驱动的进程、安全软件。
/// Windows 目录下的程序另由分类器整体排除。
const NEVER_SCHEDULED_PREFIXES: &[&str] = &[
    // 虚拟机与 WSL（WSL 的界面和转发进程）
    "wsl",
    "msrdc",
    // 输入法、触摸键盘与手写面板
    "imewatchdog",
    "tabtip",
    "sogou",
    "qqpinyin",
    "baiduime",
    "wetype",
    // 键盘鼠标等外设
    "lghub",
    "logioptionsplus",
    "razer",
    "icue",
    "corsair",
    "steelseries",
    // 显卡、声卡驱动
    "nvcontainer",
    "nvdisplay.container",
    "nvidia",
    "nvsphelper",
    "radeonsoftware",
    "amdrs",
    "igfx",
    "rtkaud",
    "nahimic",
    "dolby",
    "waves",
    // 网络
    "killer",
    // 安全软件
    "360",
    "hipstray",
    "wsctrl",
    "qqpcrtp",
    "qqpctray",
    "kxetray",
    "avp",
    "avast",
    "avg",
    "mcafee",
    "bdagent",
    "eset",
];

/// 不参与调度的安装目录（路径片段）：安全软件、输入法、驱动程序。
/// 同一厂商的程序名称多变（弹窗拦截、升级助手等），按目录判断更可靠。
const NEVER_SCHEDULED_DIRS: &[&str] = &[
    // 安全软件
    r"\huorong\",
    r"\360\",
    r"\360safe\",
    r"\qqpcmgr\",
    r"\kaspersky lab\",
    r"\avast software\",
    r"\avg\",
    r"\mcafee\",
    r"\eset\",
    r"\bitdefender\",
    r"\norton",
    r"\windows defender\",
    // 输入法
    r"\sogouinput\",
    r"\qqpinyin\",
    r"\baiduime\",
    r"\doubaoime\",
    // 驱动程序
    r"\nvidia corporation\",
    r"\realtek\",
];

/// 无论如何都不重启的系统服务。重启它们可能导致系统不稳定或降低安全性。
const PROTECTED_SERVICES: &[&str] = &[
    "rpcss",
    "dcomlaunch",
    "rpceptmapper",
    "lsm",
    "samss",
    "eventlog",
    "plugplay",
    "power",
    "brokerinfrastructure",
    "systemeventsbroker",
    "schedule",
    "winmgmt",
    "wuauserv",
    "trustedinstaller",
    "bfe",
    "mpssvc",
    "windefend",
    "wdnissvc",
    "securityhealthservice",
    "wscsvc",
    "cryptsvc",
    "keyiso",
    "vaultsvc",
    "dhcp",
    "dnscache",
    "nsi",
    "netprofm",
    "profsvc",
    "usermanager",
    "termservice",
    "sens",
    "coremessagingregistrar",
    "gpsvc",
    "appinfo",
    "staterepository",
    "msiserver",
    "lanmanworkstation",
    "lanmanserver",
    "wlansvc",
    "audioendpointbuilder",
];

pub fn is_anti_cheat_process(name: &str) -> bool {
    ANTI_CHEAT_PROCESSES.contains(&name)
}

pub fn is_anti_cheat_service(name: &str) -> bool {
    ANTI_CHEAT_SERVICES
        .iter()
        .any(|s| s.eq_ignore_ascii_case(name))
}

pub fn is_vm_process(name: &str) -> bool {
    VM_PROCESSES.contains(&name)
}

pub fn is_protected_service(name: &str) -> bool {
    PROTECTED_SERVICES
        .iter()
        .any(|s| s.eq_ignore_ascii_case(name))
}

/// 名单里不参与调度的程序（按文件名前缀）。
pub fn is_never_scheduled(name: &str) -> bool {
    NEVER_SCHEDULED_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// 名单里不参与调度的程序（按安装目录）。要逐个目录查找路径，分类时每个路径只算一次
/// （见 `classify::tree::Marks`）。
pub fn is_never_scheduled_path(path: &str) -> bool {
    NEVER_SCHEDULED_DIRS.iter().any(|d| path.contains(d))
}
