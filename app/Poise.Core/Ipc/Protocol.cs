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

public enum Mode { Quiet, Balanced, Full }

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

public sealed record DiskInfo(string Name, uint? TempC, DiskHealth Health, uint? WearPct);

public sealed record DriveSpace(ulong FreeBytes, ulong TotalBytes);

public sealed record MemoryInfo(ulong TotalBytes, uint SpeedMhz, uint Modules);

public sealed record HardwareInfo(
    string Profile,
    bool ThermalControl,
    List<FanInfo> Fans,
    List<DiskInfo> Disks,
    DriveSpace? SystemDrive,
    MemoryInfo? Memory);

public enum CleanupTrigger { Manual, Auto, Deferred }

/// <summary>最近一次真正做了事的清理。</summary>
public sealed record LastCleanup(ulong AtUnixMs, CleanupTrigger Trigger, ulong FreedBytes, int Handled);

public sealed record Status(
    Health Health,
    Mode Mode,
    Mode EffectiveMode,
    bool AutoFullForGames,
    bool AutoClean,
    Metrics Metrics,
    HardwareInfo Hardware,
    int AdjustedProcesses,
    int DeferredCleanup,
    bool DryRun,
    LastCleanup? LastCleanup);

public sealed record HelloResult(int Protocol, string ServiceVersion);

/// <summary>清理建议。kind：heavy_app / restart_recommended / manual_pending。</summary>
public sealed record Advisory(string Kind, string? Name, ulong? PrivateBytes, int? Count);

public sealed record CleanupResult(ulong FreedBytes, List<string> Handled, List<string> Failed, int Deferred, List<Advisory> Advisories);

public sealed record AppEntry(uint? Pid, string Name, string Path, AppPreference Preference, bool Running, bool IsGame);

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

/// <summary>卡顿诊断。cause.kind：thermal / background_cpu / memory_pressure / leaks / foreground_heavy / unclear。</summary>
public sealed record StutterReport(JsonElement Cause, float CpuPct, float MemUsedPct, List<JsonElement> TopCpu, List<JsonElement> TopMemory)
{
    public string CauseKind => Cause.TryGetProperty("kind", out var k) ? k.GetString() ?? "unclear" : "unclear";

    public string? CauseName => Cause.TryGetProperty("name", out var n) ? n.GetString() : null;
}

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
