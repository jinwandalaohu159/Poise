// 程序偏好页：程序列表、每个程序和进程的类别与调度状态。
namespace Poise.Core.Ipc;

/// <summary>
/// 程序偏好页的一个程序。CpuPct、MemoryBytes 为全部进程合计（未运行时为 0）；
/// Processes 为组成它的进程，主进程在前（未运行时为空）。
/// </summary>
public sealed record AppEntry(
    uint? Pid,
    string Name,
    string Path,
    AppPreference Preference,
    bool Running,
    bool IsGame,
    AppSchedule? Schedule,
    AppKind Kind,
    float CpuPct,
    ulong MemoryBytes,
    List<AppProcess> Processes);

/// <summary>
/// 程序名下的一个进程：文件名、小写路径、占整机 CPU 的百分比、专用工作集、自己的调度状态。
/// Preference 不为空时可以单独设置类别（同一程序里有自己窗口的另一个程序，例如客户端里的游戏本体）。
/// </summary>
public sealed record AppProcess(
    uint Pid,
    string Name,
    string Path,
    AppPreference? Preference,
    float CpuPct,
    ulong MemoryBytes,
    AppSchedule Schedule);

/// <summary>app：用户程序（可选择类别）；chore：系统后台杂活（移到能效核）；excluded：系统与驱动相关、不参与调度。</summary>
public enum AppKind { App, Excluded }

/// <summary>scheduled：按类别调整；excluded：不参与调度；self_managed：程序自行管理；off：智能调度已关闭。</summary>
public enum ScheduleState { Scheduled, Excluded, SelfManaged, Off }

public enum PriorityLevel { Lower, Normal, Higher, High }

/// <summary>程序当前的调度状态。Cores 为 null 表示不是大小核处理器；Adjustment 是 Poise 对它做了什么。</summary>
public sealed record AppSchedule(
    ScheduleState State,
    bool Foreground,
    CoreUse? Cores,
    PriorityLevel Priority,
    Adjustment Adjustment);

/// <summary>any：不限制；performance：固定在大核（正在玩的游戏）；efficiency：让到小核。</summary>
public enum CoreUse { Any, Performance, Efficiency }

/// <summary>none：没有调整；yielded：让路（能效核、较低优先级）；protected：不让系统节能限速；raised：标记的游戏，优先级高一级。</summary>
public enum Adjustment { None, Yielded, Protected, Raised }
