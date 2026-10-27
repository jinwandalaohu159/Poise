using Poise.Core.Ipc;
using Poise.Core.Localization;

namespace Poise.Core.Presentation;

/// <summary>设置页"这台电脑"里的一行。</summary>
public sealed record HardwareRow(string Label, string Value);

/// <summary>把硬件读数换成显示用的几行（读不到的项目不显示）。</summary>
public static class HardwareText
{
    public static IReadOnlyList<HardwareRow> Rows(Status status)
    {
        var rows = new List<HardwareRow> { new(Loc.Get("settings.device"), status.Hardware.Profile) };
        var m = status.Metrics;
        if (m.CpuTemp is { } cpu)
        {
            rows.Add(new(Loc.Get("hw.cpu_temp"), $"{cpu:0}°C"));
        }
        if (m.GpuTemp is { } gpu)
        {
            rows.Add(new(Loc.Get("hw.gpu_temp"), $"{gpu:0}°C"));
        }
        var hw = status.Hardware;
        if (hw.Fans.Count > 0)
        {
            rows.Add(new(Loc.Get("hw.fans"), string.Join(" · ", hw.Fans.Select(f => Loc.Format("hw.fan_value", f.Rpm)))));
        }
        if (hw.Memory is { } mem)
        {
            rows.Add(new(Loc.Get("hw.memory"), Loc.Format("hw.memory_value", Format.Bytes(mem.TotalBytes), mem.SpeedMhz, mem.Modules)));
        }
        foreach (var disk in hw.Disks)
        {
            rows.Add(new(disk.Name, DiskValue(disk)));
        }
        if (hw.SystemDrive is { } drive)
        {
            rows.Add(new(Loc.Get("hw.system_drive"), Loc.Format("hw.system_drive_value", Format.Bytes(drive.FreeBytes), Format.Bytes(drive.TotalBytes))));
        }
        rows.Add(new(Loc.Get("settings.thermal"), Loc.Get(hw.ThermalControl ? "settings.thermal.on" : "settings.thermal.off")));
        return rows;
    }

    private static string DiskValue(DiskInfo d)
    {
        var parts = new List<string>();
        if (d.TempC is { } t)
        {
            parts.Add($"{t}°C");
        }
        parts.Add(Loc.Get(d.Health switch
        {
            DiskHealth.Healthy => "hw.disk.healthy",
            DiskHealth.Warning => "hw.disk.warning",
            DiskHealth.Unhealthy => "hw.disk.unhealthy",
            _ => "hw.disk.unknown",
        }));
        if (d.WearPct is { } w)
        {
            parts.Add(Loc.Format("hw.disk_wear", w));
        }
        return string.Join(" · ", parts);
    }
}
