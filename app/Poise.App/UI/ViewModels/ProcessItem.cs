using CommunityToolkit.Mvvm.ComponentModel;
using Poise.Core.Ipc;
using Poise.Core.Presentation;

namespace Poise.App.UI.ViewModels;

/// <summary>程序展开后的一个进程：文件名、调度状态、CPU、内存。只供查看，偏好按程序设置。</summary>
public sealed partial class ProcessItem : ObservableObject
{
    public ProcessItem(AppProcess p)
    {
        Pid = p.Pid;
        Name = p.Name;
        Update(p);
    }

    public uint Pid { get; }

    public string Name { get; }

    /// <summary>进程号会被系统复用，加上文件名区分。</summary>
    public string Key => KeyOf(Pid, Name);

    internal static string KeyOf(AppProcess p) => KeyOf(p.Pid, p.Name);

    private static string KeyOf(uint pid, string name) => $"{pid}:{name}";

    [ObservableProperty]
    public partial string Detail { get; private set; } = "";

    [ObservableProperty]
    public partial string Cpu { get; private set; } = "";

    [ObservableProperty]
    public partial string Memory { get; private set; } = "";

    internal void Update(AppProcess p)
    {
        Detail = ScheduleText.State(p.Schedule) ?? "";
        Cpu = AppsText.Cpu(p.CpuPct);
        Memory = AppsText.Memory(p.MemoryBytes);
    }
}
