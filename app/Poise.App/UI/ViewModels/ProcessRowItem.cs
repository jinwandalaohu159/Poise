using CommunityToolkit.Mvvm.ComponentModel;
using Microsoft.UI.Xaml;
using Poise.Core.Presentation;

namespace Poise.App.UI.ViewModels;

/// <summary>清理页的一行。数据刷新时原地更新，不重建，列表不跳动。</summary>
public sealed partial class ProcessRowItem : ObservableObject
{
    public ProcessRowItem(ProcessRowView row) => Update(row);

    public uint Pid { get; private set; }

    public string? Path { get; private set; }

    [ObservableProperty] public partial string Name { get; private set; } = "";
    [ObservableProperty, NotifyPropertyChangedFor(nameof(CountVisibility))]
    public partial string ProcessCount { get; private set; } = "";
    [ObservableProperty] public partial ulong MemoryBytes { get; private set; }
    [ObservableProperty] public partial string Memory { get; private set; } = "";
    [ObservableProperty] public partial double MemoryFraction { get; private set; }
    [ObservableProperty] public partial string Cpu { get; private set; } = "";
    [ObservableProperty] public partial string Status { get; private set; } = "";
    [ObservableProperty] public partial TagTone Tone { get; private set; }
    [ObservableProperty, NotifyPropertyChangedFor(nameof(ActionVisibility))]
    public partial string Action { get; private set; } = "";
    [ObservableProperty] public partial bool NeedsAttention { get; private set; }

    public Visibility CountVisibility => ProcessCount.Length > 0 ? Visibility.Visible : Visibility.Collapsed;

    public Visibility ActionVisibility => Action.Length > 0 ? Visibility.Visible : Visibility.Collapsed;

    internal void Update(ProcessRowView row)
    {
        Pid = row.Pid;
        Path = row.Path;
        Name = row.Name;
        ProcessCount = row.ProcessCount;
        MemoryBytes = row.MemoryBytes;
        Memory = row.Memory;
        MemoryFraction = row.MemoryFraction;
        Cpu = row.Cpu;
        Status = row.Status;
        Tone = row.Tone;
        Action = row.Action;
        NeedsAttention = row.NeedsAttention;
    }
}
