//! 参考名单。Poise 不靠名单决定"做什么"，这里只有两类用途：
//! 1. 安全：确保反作弊、虚拟机、关键系统服务永远不被触碰；
//! 2. 识别：根据安装目录判断前台全屏程序是不是游戏。
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
