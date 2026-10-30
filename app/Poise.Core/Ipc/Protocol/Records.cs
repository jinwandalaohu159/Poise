// 使用记录：每段有重点程序的时间（玩游戏、编译、渲染……）一条。
namespace Poise.Core.Ipc;

/// <summary>
/// 一段有重点程序的时间。Programs 是这段时间里的重点程序（作为重点程序的时间长的在前）；Modes 是用过的模式；
/// MovedAsidePeak 是同一时刻最多有几个程序被挪到小核；YieldedPrograms 是让过路的后台程序个数；
/// 温度读不到时为 null；ThrottledSecs 是处理器被限速的累计秒数。
/// </summary>
public sealed record SessionRecord(
    ulong StartedUnixMs,
    ulong EndedUnixMs,
    List<RecordProgram> Programs,
    List<Mode> Modes,
    uint MovedAsidePeak,
    uint YieldedPrograms,
    float CpuPctAvg,
    float CpuPctPeak,
    float? CpuTempAvg,
    float? CpuTempPeak,
    uint ThrottledSecs);

/// <summary>
/// 记录里的一个重点程序。PeakLoadPct 是最高负载（单核百分比），WidestDomains 是用过的最多的核心域（第 n 位是第 n 个域）。
/// </summary>
public sealed record RecordProgram(string Name, string? Path, bool Game, uint FocusedSecs, uint PeakLoadPct, ushort WidestDomains);
