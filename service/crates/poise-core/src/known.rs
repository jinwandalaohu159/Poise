//! 参考名单。Poise 不靠名单决定"做什么"，这里只有三类用途：
//! 1. 安全：确保反作弊、虚拟机、关键系统服务，以及输入、显示、声音、安全软件相关的程序永远不被触碰；
//! 2. 识别：根据安装目录判断前台全屏程序是不是游戏；
//! 3. 后台杂活：明确属于索引、更新、遥测评估的系统与软件进程（以及只承载这类服务的服务进程），调度时让到小核。
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

/// 后台杂活：只挪到小核并开启节能，不改优先级。`svchost` 不在其中（一个进程承载多个服务）。
const BACKGROUND_CHORES: &[&str] = &[
    // 文件搜索索引
    "searchindexer.exe",
    "searchprotocolhost.exe",
    "searchfilterhost.exe",
    // Windows 更新的工作进程
    "mousocoreworker.exe",
    "usoclient.exe",
    "tiworker.exe",
    "wuauclt.exe",
    // 兼容性与遥测评估
    "compattelrunner.exe",
    "devicecensus.exe",
    // 软件自动更新
    "googleupdate.exe",
    "microsoftedgeupdate.exe",
    "onedrivestandaloneupdater.exe",
    "adobearm.exe",
];

/// 后台杂活服务（服务名，小写）：只在进程只承载这一个服务时才调整，避免连带同进程的其他服务。
/// 预读（SysMain）、Office 即点即用等会影响前台体验的服务不在其中。
const BACKGROUND_CHORE_SERVICES: &[&str] = &[
    // Windows 更新：检查、下载、安装与编排
    "wuauserv",
    "usosvc",
    // 后台下载：后台智能传送、传递优化
    "bits",
    "dosvc",
    // 诊断与遥测、兼容性助手
    "diagtrack",
    "pcasvc",
    // Microsoft Store 安装服务
    "installservice",
];

/// 典型的游戏安装目录片段。
const GAME_DIR_HINTS: &[&str] = &[
    r"\steamapps\common\",
    r"\epic games\",
    r"\riot games\",
    r"\wegameapps\",
    r"\gog galaxy\games\",
    r"\xboxgames\",
    r"\mihoyo\",
    r"\hoyoplay\games\",
    r"\ubisoft game launcher\games\",
    r"\ea games\",
    r"\netease\",
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
    ANTI_CHEAT_SERVICES.contains(&name.to_lowercase().as_str())
}

pub fn is_vm_process(name: &str) -> bool {
    VM_PROCESSES.contains(&name)
}

pub fn looks_like_game_path(path: Option<&str>) -> bool {
    path.is_some_and(|p| GAME_DIR_HINTS.iter().any(|h| p.contains(h)))
}

pub fn is_protected_service(name: &str) -> bool {
    PROTECTED_SERVICES.contains(&name.to_lowercase().as_str())
}

pub fn is_never_scheduled(name: &str) -> bool {
    NEVER_SCHEDULED_PREFIXES.iter().any(|p| name.starts_with(p))
}

pub fn is_never_scheduled_path(path: &str) -> bool {
    NEVER_SCHEDULED_DIRS.iter().any(|d| path.contains(d))
}

pub fn is_background_chore_service(name: &str) -> bool {
    BACKGROUND_CHORE_SERVICES.contains(&name.to_lowercase().as_str())
}

pub fn is_background_chore(name: &str) -> bool {
    BACKGROUND_CHORES.contains(&name)
}
