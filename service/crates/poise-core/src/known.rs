//! 参考名单。Poise 不靠名单决定"做什么"（游戏按行为识别，系统进程一律不碰），这里只用于安全：
//! 确保反作弊、虚拟机、关键系统服务，以及输入、显示、声音、安全软件相关的程序永远不被触碰；
//! 唯一的例外是 Windows 自己的几个维护程序（见 [`is_maintenance_worker`]），只允许把它们挪开；
//!
//! 所有名称均为小写。

/// 反作弊程序的进程名。
const ANTI_CHEAT_PROCESSES: &[&str] = &[
    "vgc.exe",
    "vgm.exe",
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

/// 反作弊软件的安装目录名（路径里的一段）。同一套反作弊往往有好几个程序，装在这些目录里的一律算反作弊。
const ANTI_CHEAT_FOLDERS: &[&str] = &[
    "riot vanguard",
    "easyanticheat",
    "easyanticheat_eos",
    "battleye",
    "anticheatexpert",
    "faceit ac",
    "gameguard",
    "xigncode3",
    "nprotect",
    "punkbuster",
];

/// 虚拟机工作进程，包括安卓模拟器和桌面虚拟机：它们由后台服务拉起、没有窗口，和显示画面的那个窗口
/// 不在一个进程树里——当成普通后台程序让路的话，真正在跑游戏的正是它。
const VM_PROCESSES: &[&str] = &[
    "vmmem",
    "vmmemwsl",
    "vmwp.exe",
    "vmcompute.exe",
    "vmms.exe",
    // 雷电、MuMu、夜神、逍遥模拟器
    "ld9boxheadless.exe",
    "ldboxheadless.exe",
    "ldvboxheadless.exe",
    "mumuvmmheadless.exe",
    "nemuheadless.exe",
    "noxvmhandle.exe",
    "memuheadless.exe",
    // VMware、VirtualBox
    "vmware-vmx.exe",
    "vboxheadless.exe",
    "virtualboxvm.exe",
];

/// 用户会话里不参与调度的程序（名称前缀）：输入法、外设助手、显卡与声卡驱动的进程、安全软件。
/// Windows 目录下的程序另由分类器整体排除。前缀要足够长：太短会把同名开头的游戏和程序也排除掉
/// （例如"360"会排除 360 浏览器，它就永远不让路）。
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
    "wavessvc",
    "wavessyssvc",
    // 网络
    "killernetwork",
    // 安全软件
    "360tray",
    "360sd",
    "360rp",
    "zhudongfangyu",
    "hipstray",
    "wsctrl",
    "qqpcrtp",
    "qqpctray",
    "kxetray",
    "avp.",
    "avpui",
    "avast",
    "avgui",
    "avgsvc",
    "avgtools",
    "mcafee",
    "bdagent",
    "ekrn",
    "egui",
];

/// 不参与调度的安装目录（路径片段）：安全软件、输入法、驱动程序。
/// 同一厂商的程序名称多变（弹窗拦截、升级助手等），按目录判断更可靠。
const NEVER_SCHEDULED_DIRS: &[&str] = &[
    // 安全软件
    r"\huorong\",
    r"\360safe\",
    r"\360sd\",
    r"\360total security\",
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

/// 游戏库目录（路径片段）：装在这里的程序是启动器打开的游戏，不是启动器的一部分
/// （Steam 的游戏默认装在 Steam 目录里面）。
const GAME_LIBRARY_DIRS: &[&str] = &[
    r"\steamapps\common\",
    r"\gog galaxy\games\",
    r"\ubisoft game launcher\games\",
];

/// Windows 自己的维护程序：更新的下载与安装、搜索索引、兼容性评估、.NET 预编译、恶意软件删除工具、
/// 磁盘优化。它们在后台不定时地大量占用处理器和磁盘，玩游戏、干重活时常因此卡一下。
/// 只认 Windows 目录下的（普通程序写不进 Windows 目录，冒不了名），见 `classify::ScheduleScope::Maintenance`。
const MAINTENANCE_WORKERS: &[&str] = &[
    "tiworker.exe",
    "mousocoreworker.exe",
    "usocoreworker.exe",
    "wuauclt.exe",
    "sihclient.exe",
    "searchindexer.exe",
    "searchprotocolhost.exe",
    "searchfilterhost.exe",
    "compattelrunner.exe",
    "devicecensus.exe",
    "dismhost.exe",
    "mrt.exe",
    "mscorsvw.exe",
    "ngen.exe",
    "ngentask.exe",
    "defrag.exe",
];

/// 装在 Program Files 里、以服务身份在后台更新的程序（Office 即点即用、Edge 与 Chrome 的更新、
/// Firefox 的维护服务、Adobe 的更新）：和 Windows 的维护程序一样，有重点程序时只挪开（路径片段）。
/// 只认 Program Files 下的（普通程序写不进去）。
const MAINTENANCE_DIRS: &[&str] = &[
    r"\common files\microsoft shared\clicktorun\",
    r"\microsoft\edgeupdate\",
    r"\google\update\",
    r"\google\googleupdater\",
    r"\mozilla maintenance service\",
    r"\adobe\arm\",
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

/// 路径（小写）里有一段是已知的反作弊目录。
pub fn is_anti_cheat_path(path: &str) -> bool {
    path.split('\\')
        .rev()
        .skip(1)
        .any(|segment| ANTI_CHEAT_FOLDERS.contains(&segment))
}

pub fn is_anti_cheat_service(name: &str) -> bool {
    ANTI_CHEAT_SERVICES
        .iter()
        .any(|s| s.eq_ignore_ascii_case(name))
}

/// Windows 自己的维护程序（按文件名；调用方还要确认它在 Windows 目录下）。
pub fn is_maintenance_worker(name: &str) -> bool {
    MAINTENANCE_WORKERS.contains(&name)
}

/// 装在 Program Files 里的后台更新程序（路径小写，见 [`MAINTENANCE_DIRS`]）。
pub fn is_maintenance_path(path: &str) -> bool {
    path.contains(r":\program files") && MAINTENANCE_DIRS.iter().any(|d| path.contains(d))
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

/// 装在游戏库目录里（见 [`GAME_LIBRARY_DIRS`]）。
pub fn is_in_game_library(path: &str) -> bool {
    GAME_LIBRARY_DIRS.iter().any(|d| path.contains(d))
}

/// 名单里不参与调度的程序（按安装目录）。要逐个目录查找路径，分类时每个路径只算一次
/// （见 `classify::tree::Marks`）。
pub fn is_never_scheduled_path(path: &str) -> bool {
    NEVER_SCHEDULED_DIRS.iter().any(|d| path.contains(d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_never_scheduled_list_does_not_catch_programs_with_similar_names() {
        assert!(is_never_scheduled("360tray.exe"));
        assert!(is_never_scheduled("avp.exe"));
        // 360 浏览器、名字以 avg 开头的游戏照常调度。
        assert!(!is_never_scheduled("360se.exe"));
        assert!(!is_never_scheduled("360chromex.exe"));
        assert!(!is_never_scheduled("avgame.exe"));
        assert!(!is_never_scheduled("killer instinct.exe"));
        assert!(!is_never_scheduled_path(
            r"c:\program files (x86)\360\360se6\application\360se.exe"
        ));
        assert!(is_never_scheduled_path(
            r"c:\program files (x86)\360\360safe\360tray.exe"
        ));
    }

    #[test]
    fn games_in_a_launcher_library_are_recognized() {
        assert!(is_in_game_library(
            r"c:\program files (x86)\steam\steamapps\common\dota 2 beta\game\bin\win64\dota2.exe"
        ));
        assert!(!is_in_game_library(
            r"c:\program files (x86)\steam\bin\cef\cef.win7x64\steamwebhelper.exe"
        ));
    }
}
