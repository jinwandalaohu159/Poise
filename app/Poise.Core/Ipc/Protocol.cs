// 与 Rust 端 poise-protocol 一一对应的数据结构。字段名统一为 snake_case。
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Poise.Core.Ipc;

public static class ProtocolJson
{
    public const int Version = 1;

    public static readonly JsonSerializerOptions Options = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        DefaultIgnoreCondition = JsonIgnoreCondition.Never,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.SnakeCaseLower) },
    };
}

/// <summary>模式。实际生效的模式不会是 Auto；Vendor 是机型极限模式（戴尔 G 模式等）。</summary>
public enum Mode { Auto, Quiet, Balanced, Full, Vendor }

/// <summary>玩游戏时切换到哪个模式。</summary>
public enum GameMode { Off, Full, Vendor }

public enum AppPreference { Auto, AlwaysFast, Background, Game }

public enum HealthCode { Smooth, InGame, Tired, Hot, RestartRecommended, StorageLow }

public enum Level { Good, Notice, Bad }

public sealed record Health(HealthCode Code, Level Level, string? Game, int Cleanable, ulong FreeableBytes);

public sealed record Metrics(
    float CpuPct,
    float? CpuTemp,
    float? GpuTemp,
    float? GpuPct,
    float MemUsedPct,
    ulong MemUsedBytes,
    ulong MemTotalBytes,
    uint CpuMhz,
    bool OnBattery);

public sealed record FanInfo(uint Rpm, uint MaxRpm);

public enum DiskHealth { Healthy, Warning, Unhealthy, Unknown }

/// <summary>一块硬盘：型号、容量、温度、健康、寿命已用，以及有盘符的分区。</summary>
public sealed record DiskInfo(string Name, ulong? SizeBytes, uint? TempC, DiskHealth Health, uint? WearPct, List<VolumeInfo> Volumes);

/// <summary>有盘符的分区，Letter 例如 "C"。</summary>
public sealed record VolumeInfo(string Letter, ulong FreeBytes, ulong TotalBytes);

public sealed record DriveSpace(ulong FreeBytes, ulong TotalBytes);

/// <summary>内存：总量、频率、每条的容量。</summary>
public sealed record MemoryInfo(ulong TotalBytes, uint SpeedMhz, List<ulong> ModuleBytes);

public sealed record HardwareInfo(
    string Profile,
    bool ThermalControl,
    bool FanBoostSupported,
    /// <summary>机型极限模式的标识（例如 "dell_g_mode"）；不支持时为 null。</summary>
    string? VendorMode,
    List<FanInfo> Fans,
    List<DiskInfo> Disks,
    DriveSpace? SystemDrive,
    MemoryInfo? Memory,
    CpuInfo? Cpu);

/// <summary>处理器的核心组成。不是大小核处理器时，全部核心算在"大核"里，小核为 0。</summary>
public sealed record CpuInfo(uint PerformanceCores, uint PerformanceThreads, uint EfficiencyCores, uint EfficiencyThreads);

public enum CleanupTrigger { Manual, Auto, Deferred }

/// <summary>最近一次真正做了事的清理。</summary>
public sealed record LastCleanup(ulong AtUnixMs, CleanupTrigger Trigger, ulong FreedBytes, int Handled);

public sealed record Status(
    Health Health,
    Mode Mode,
    Mode EffectiveMode,
    GameMode GameMode,
    bool AutoClean,
    /// <summary>风扇加速设置，0～100（0 = 不加速，由 BIOS 按温度调节）。</summary>
    int FanBoost,
    bool SmartSchedule,
    Metrics Metrics,
    HardwareInfo Hardware,
    int AdjustedProcesses,
    int DeferredCleanup,
    bool DryRun,
    LastCleanup? LastCleanup);

public sealed record HelloResult(int Protocol, string ServiceVersion);

/// <summary>清理建议。kind：heavy_app / restart_recommended / manual_pending。</summary>
public sealed record Advisory(string Kind, string? Name, ulong? MemoryBytes, int? Count);

public sealed record CleanupResult(ulong FreedBytes, List<string> Handled, List<string> Failed, int Deferred, List<Advisory> Advisories);

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

/// <summary>程序名下的一个进程：文件名、占整机 CPU 的百分比、专用工作集、自己的调度状态。</summary>
public sealed record AppProcess(uint Pid, string Name, float CpuPct, ulong MemoryBytes, AppSchedule Schedule);

/// <summary>app：用户程序（可选择类别）；chore：系统后台杂活（移到能效核）；excluded：系统与驱动相关、不参与调度。</summary>
public enum AppKind { App, Chore, Excluded }

/// <summary>scheduled：按类别调整；excluded：不参与调度；self_managed：程序自行管理；off：智能调度已关闭。</summary>
public enum ScheduleState { Scheduled, Excluded, SelfManaged, Off }

public enum PriorityLevel { Lower, Normal, Higher, High }

/// <summary>程序当前的调度状态。SmallCores 为 null 表示不是大小核处理器。</summary>
public sealed record AppSchedule(ScheduleState State, bool Foreground, bool? SmallCores, PriorityLevel Priority);

/// <summary>文案：type 为 key（本地化键）或 plain（原文）。</summary>
public sealed record TextValue(string Type, string Value);

public sealed record OptimizationItem(
    string Id,
    string Group,
    TextValue Title,
    TextValue Description,
    bool InOneClick,
    bool SlowHint,
    string TakesEffect);

public sealed record ItemStatus(OptimizationItem Item, string State, bool Revertible)
{
    public bool IsOptimized => State == "optimized";
}

public sealed record ApplyReport(List<string> Applied, List<JsonElement> Failed);

/// <summary>
/// 走势上的一个点（SecsAgo 越大越早）。PerformancePct、EfficiencyPct 是大核、小核的平均占用，
/// 不是大小核处理器时为 null。
/// </summary>
public sealed record HistoryPoint(float SecsAgo, float CpuPct, float? PerformancePct, float? EfficiencyPct, float MemPct, float? CpuTemp);

/// <summary>最近几分钟的走势：横轴秒数与从早到晚的点（比横轴多几秒）。</summary>
public sealed record Trend(uint WindowSecs, List<HistoryPoint> Points);

/// <summary>
/// 卡顿诊断：结论（cause.kind：thermal / background_cpu / memory_pressure / leaks / foreground_heavy / unclear）与依据。
/// </summary>
public sealed record Diagnosis(
    JsonElement Cause,
    float CpuPct,
    float MemUsedPct,
    float? CpuTemp,
    bool Throttling,
    List<JsonElement> TopCpu,
    List<JsonElement> TopMemory,
    bool SmartSchedule)
{
    public string CauseKind => Text("kind") ?? "unclear";

    public string? CauseName => Text("name");

    /// <summary>后台程序的路径（用于"设为低优先级"）。</summary>
    public string? CausePath => Text("path");

    /// <summary>该程序占整机 CPU 的百分比。</summary>
    public float? CauseShare => Cause.TryGetProperty("share_pct", out var v) && v.ValueKind == JsonValueKind.Number ? v.GetSingle() : null;

    public int? CauseCount => Cause.TryGetProperty("count", out var v) && v.ValueKind == JsonValueKind.Number ? v.GetInt32() : null;

    /// <summary>占用 CPU 最多的程序：名称与占整机的百分比。</summary>
    public IEnumerable<(string Name, double Pct)> CpuRanking =>
        TopCpu.Select(e => (e[0].GetString() ?? "", e[1].GetDouble()));

    /// <summary>占用内存最多的程序：名称与实际占用的物理内存。</summary>
    public IEnumerable<(string Name, ulong Bytes)> MemoryRanking =>
        TopMemory.Select(e => (e[0].GetString() ?? "", e[1].GetUInt64()));

    private string? Text(string name) =>
        Cause.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;
}

/// <summary>accumulated：句柄、线程或内存比进程稳定运行时多出数倍。</summary>
public enum ProcessIssue { None, Growing, Accumulated, Oversized, Heavy }

public enum GrowthMetric { Handles, Threads, Memory }

/// <summary>某一项从基线到现在的变化（内存为字节，其余为个数）。</summary>
public sealed record Change(GrowthMetric Metric, ulong From, ulong To);

/// <summary>一键清理会怎样处理。kind：none / restart_service / recreate_compositor / restart_app / reopen_manually。</summary>
public sealed record Remedy(string Kind, string? Service);

/// <summary>清理页的一行：同一程序的进程合并为一行。</summary>
public sealed record ProcessGroup(
    uint Pid,
    string Name,
    string? Path,
    int ProcessCount,
    /// <summary>实际占用的物理内存（专用工作集，与任务管理器一致）。</summary>
    ulong MemoryBytes,
    float CpuPct,
    ProcessIssue Issue,
    ulong? GrowthPerHour,
    Change? SinceStart,
    Remedy Remedy,
    bool Disruptive);

/// <summary>整机总量的一项。kernel_pool：内核分页池与非分页池之和。</summary>
public enum TotalItem { Handles, Threads, Processes, Commit, KernelPool, SystemCache }

/// <summary>与开机时相比的一项。Notable：增长明显超出正常使用。</summary>
public sealed record TotalChange(TotalItem Item, ulong AtBoot, ulong Now, bool Notable);

public sealed record SinceBoot(ulong BaselineUptimeMs, ulong BaselineAtUnixMs, ulong UptimeMs, List<TotalChange> Items);

public sealed record CleanupOverview(
    ulong FreeableBytes,
    int NeedsAttention,
    ulong MemUsedBytes,
    ulong MemTotalBytes,
    bool RestartAdvised,
    bool AutoClean,
    LastCleanup? LastCleanup,
    /// <summary>开机后系统还没稳定（约 5 分钟）时为空。</summary>
    SinceBoot? SinceBoot,
    List<ProcessGroup> Processes);

/// <summary>服务返回的失败分类。界面按它显示对应文案，服务给的文字只写进日志。</summary>
public enum ErrorCode
{
    Unknown,
    InvalidRequest,
    VersionMismatch,
    ObserveOnly,
    NotFound,
    ProcessGone,
    InvalidPath,
    NotReady,
    Failed,
    Internal,
}
