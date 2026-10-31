// 状态：主页与托盘面板（健康、模式、读数、硬件信息）。
namespace Poise.Core.Ipc;

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
    LastCleanup? LastCleanup,
    /// <summary>暂停着：托盘代理没在上报（没运行、连不上服务），调整已撤销、不调度。</summary>
    bool Paused = false);

public sealed record HelloResult(int Protocol, string ServiceVersion);
