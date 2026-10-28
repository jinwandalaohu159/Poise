using Poise.Core.Ipc;
using Poise.Core.Localization;

namespace Poise.Core.Presentation;

/// <summary>设置页"硬件信息"里的一行：名称在左，数值在右。</summary>
public sealed record HardwareRow(string Label, string Value);

/// <summary>把硬件读数换成显示用的几行（读不到的项目不显示）。</summary>
public static class HardwareText
{
    public static IReadOnlyList<HardwareRow> Rows(Status status)
    {
        var hw = status.Hardware;
        var m = status.Metrics;
        var rows = new List<HardwareRow> { new(Loc.Get("settings.device"), hw.Profile) };
        if (hw.Cpu is { } processor)
        {
            rows.Add(new(Loc.Get("hw.cpu"), Processor(processor)));
        }
        if (m.CpuTemp is { } cpu)
        {
            rows.Add(new(Loc.Get("hw.cpu_temp"), $"{cpu:0}°C"));
        }
        if (m.GpuTemp is { } gpu)
        {
            rows.Add(new(Loc.Get("hw.gpu_temp"), $"{gpu:0}°C"));
        }
        if (FanText.Speed(hw.Fans) is { } fans)
        {
            rows.Add(new(Loc.Get("hw.fans"), fans));
        }
        if (hw.Memory is { } mem)
        {
            rows.Add(new(Loc.Get("hw.memory"), Memory(mem)));
        }
        rows.AddRange(Storage(hw));
        rows.Add(new(Loc.Get("settings.thermal"), Loc.Get(hw.ThermalControl ? "settings.thermal.on" : "settings.thermal.off")));
        return rows;
    }

    /// <summary>"32 GB · 16 GB × 2 · 4800 MHz"；容量不同时逐条列出："24 GB · 16 GB + 8 GB"。</summary>
    /// <summary>"6 个性能核（12 线程）+ 8 个能效核（8 线程）"；不是大小核处理器时为"8 核（16 线程）"。</summary>
    internal static string Processor(CpuInfo cpu) =>
        cpu.EfficiencyThreads == 0
            ? Loc.Format("hw.cpu.uniform", cpu.PerformanceCores, cpu.PerformanceThreads)
            : Loc.Format("hw.cpu.hybrid", cpu.PerformanceCores, cpu.PerformanceThreads, cpu.EfficiencyCores, cpu.EfficiencyThreads);

    internal static string Memory(MemoryInfo mem)
    {
        var parts = new List<string> { Format.Capacity(mem.TotalBytes) };
        var sizes = mem.ModuleBytes;
        if (sizes.Count > 0)
        {
            parts.Add(sizes.Distinct().Count() == 1
                ? $"{Format.Capacity(sizes[0])} × {sizes.Count}"
                : string.Join(" + ", sizes.OrderByDescending(b => b).Select(Format.Capacity)));
        }
        if (mem.SpeedMhz > 0)
        {
            parts.Add($"{mem.SpeedMhz} MHz");
        }
        return string.Join(" · ", parts);
    }

    /// <summary>每块硬盘一行（型号、已用 / 容量、健康），其下每个盘符一行（可用 / 总大小）。</summary>
    private static IEnumerable<HardwareRow> Storage(HardwareInfo hw)
    {
        var numbered = hw.Disks.Count > 1;
        for (var i = 0; i < hw.Disks.Count; i++)
        {
            var disk = hw.Disks[i];
            yield return new(numbered ? Loc.Format("hw.disk_n", i + 1) : Loc.Get("hw.disk"), DiskValue(disk));
            foreach (var v in disk.Volumes)
            {
                yield return new(VolumeLabel(v.Letter), Loc.Format("hw.system_drive_value", Format.Capacity(v.FreeBytes), Format.Capacity(v.TotalBytes)));
            }
        }
        // 读不到分区时（例如没有存储管理接口），至少显示系统盘。
        if (hw.SystemDrive is { } drive && hw.Disks.All(d => d.Volumes.Count == 0))
        {
            yield return new(Loc.Get("hw.system_drive"), Loc.Format("hw.system_drive_value", Format.Capacity(drive.FreeBytes), Format.Capacity(drive.TotalBytes)));
        }
    }

    private static string VolumeLabel(string letter)
    {
        var system = Environment.GetEnvironmentVariable("SystemDrive") ?? "C:";
        return system.StartsWith(letter, StringComparison.OrdinalIgnoreCase)
            ? Loc.Format("hw.volume_system", letter)
            : Loc.Format("hw.volume", letter);
    }

    internal static string DiskValue(DiskInfo d)
    {
        var parts = new List<string> { d.Name };
        if (d.SizeBytes is { } size)
        {
            var used = d.Volumes.Aggregate(0UL, (sum, v) => sum + (v.TotalBytes - Math.Min(v.FreeBytes, v.TotalBytes)));
            parts.Add(d.Volumes.Count > 0
                ? Loc.Format("hw.disk_usage", Format.Capacity(used), Format.Capacity(size))
                : Format.Capacity(size));
        }
        parts.Add(Loc.Get(d.Health switch
        {
            DiskHealth.Healthy => "hw.disk.healthy",
            DiskHealth.Warning => "hw.disk.warning",
            DiskHealth.Unhealthy => "hw.disk.unhealthy",
            _ => "hw.disk.unknown",
        }));
        if (d.TempC is { } t)
        {
            parts.Add($"{t}°C");
        }
        if (d.WearPct is { } w)
        {
            parts.Add(Loc.Format("hw.disk_wear", w));
        }
        return string.Join(" · ", parts);
    }
}
