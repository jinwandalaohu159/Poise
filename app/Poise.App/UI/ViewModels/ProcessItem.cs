using CommunityToolkit.Mvvm.ComponentModel;
using Microsoft.UI.Xaml;
using Poise.Core.Ipc;
using Poise.Core.Presentation;

namespace Poise.App.UI.ViewModels;

/// <summary>
/// 程序展开后的一个进程：文件名、调度状态、CPU、内存。一般跟随所属程序的类别；
/// 同一程序里有自己窗口的另一个程序（例如客户端里的游戏本体）可以单独设置类别，设置后它成为独立的程序。
/// </summary>
public sealed partial class ProcessItem : ObservableObject
{
    private readonly AppsViewModel _owner;
    private bool _syncing;

    public ProcessItem(AppsViewModel owner, AppProcess p)
    {
        _owner = owner;
        Pid = p.Pid;
        Name = p.Name;
        Path = p.Path;
        Update(p);
    }

    public uint Pid { get; }

    public string Name { get; }

    public string Path { get; }

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

    /// <summary>可以单独设置类别时显示类别选项。</summary>
    [ObservableProperty]
    [NotifyPropertyChangedFor(nameof(PreferenceVisibility))]
    public partial bool CanSetPreference { get; private set; }

    public Visibility PreferenceVisibility => CanSetPreference ? Visibility.Visible : Visibility.Collapsed;

    /// <summary><see cref="AppPreference"/> 的数值。</summary>
    [ObservableProperty]
    public partial int PreferenceIndex { get; set; }

    partial void OnPreferenceIndexChanged(int oldValue, int newValue)
    {
        if (_syncing || newValue < 0 || !CanSetPreference)
        {
            return;
        }
        _ = _owner.SetPreferenceAsync(Path, (AppPreference)newValue, () => Restore((AppPreference)oldValue));
    }

    /// <summary>设置显示的类别但不发请求。</summary>
    private void Restore(AppPreference value)
    {
        _syncing = true;
        PreferenceIndex = (int)value;
        _syncing = false;
    }

    internal void Update(AppProcess p)
    {
        CanSetPreference = p.Preference is not null;
        Restore(p.Preference ?? AppPreference.Auto);
        Detail = ScheduleText.State(p.Schedule) ?? "";
        Cpu = AppsText.Cpu(p.CpuPct);
        Memory = AppsText.Memory(p.MemoryBytes);
    }
}
