using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using Microsoft.UI.Xaml;
using Poise.Core.Collections;
using Poise.Core.Ipc;
using Poise.Core.Localization;
using Poise.Core.Presentation;

namespace Poise.App.UI.ViewModels;

/// <summary>
/// 程序列表中的一个程序（可展开到它的各个进程）。<see cref="PreferenceIndex"/> 即 <see cref="AppPreference"/> 的数值；
/// <see cref="Detail"/> 是正在运行的程序的调度状态（多个进程时带进程数），没有运行时是路径。
/// 刷新时原地更新，不重建：展开状态、图标都保留。
/// </summary>
public sealed partial class AppItem : ObservableObject
{
    private readonly AppsViewModel _owner;
    private List<AppProcess> _processes = [];
    private bool _everExpanded;
    private bool _syncing;

    public AppItem(AppsViewModel owner, AppEntry entry)
    {
        _owner = owner;
        Name = entry.Name;
        Path = entry.Path;
        Running = entry.Running;
        Kind = entry.Kind;
        Key = KeyOf(entry);
        Update(entry);
    }

    public string Name { get; }

    public string Path { get; }

    public bool Running { get; }

    /// <summary>用户程序可以选择类别；系统一栏（后台杂活、不参与调度的程序）只供查看。</summary>
    public AppKind Kind { get; }

    /// <summary>列表里的唯一标识：系统服务进程的路径相同，按服务名称区分。</summary>
    public string Key { get; }

    /// <summary>与 <see cref="Key"/> 相同的规则，用于把服务返回的条目对应到已有的行。</summary>
    internal static string KeyOf(AppEntry e) => KeyOf(e.Kind, e.Path, e.Name);

    private static string KeyOf(AppKind kind, string path, string name) => kind == AppKind.App ? path : $"{path}|{name}";

    /// <summary>可以设置类别（系统一栏只供查看）：不能时类别下拉框不创建。</summary>
    public bool CanSetPreference => Kind == AppKind.App;

    [ObservableProperty]
    public partial int PreferenceIndex { get; set; }

    [ObservableProperty]
    public partial string Detail { get; set; } = "";

    /// <summary>全部进程合计（排序用）。</summary>
    public float CpuPct { get; private set; }

    public ulong MemoryBytes { get; private set; }

    /// <summary>没有运行时为空。</summary>
    [ObservableProperty]
    public partial string Cpu { get; private set; } = "";

    [ObservableProperty]
    public partial string Memory { get; private set; } = "";

    /// <summary>有多个进程时可以展开。</summary>
    [ObservableProperty]
    public partial bool CanExpand { get; private set; }

    [ObservableProperty]
    public partial bool IsExpanded { get; set; }

    /// <summary>
    /// 各个进程。第一次展开时才生成，之后随刷新更新（收起后保留，收起动画期间内容不消失）。
    /// </summary>
    public ObservableCollection<ProcessItem> Processes { get; } = [];

    public Visibility ChevronVisibility => CanExpand ? Visibility.Visible : Visibility.Collapsed;

    public string ExpandLabel => Loc.Get(IsExpanded ? "apps.collapse" : "apps.expand");

    /// <summary>名称或其中任一进程名包含 <paramref name="query"/>；只有进程名匹配时 <paramref name="byProcess"/> 为真。</summary>
    internal bool Matches(string query, out bool byProcess)
    {
        byProcess = false;
        if (query.Length == 0 || Name.Contains(query, StringComparison.CurrentCultureIgnoreCase))
        {
            return true;
        }
        byProcess = _processes.Any(p => p.Name.Contains(query, StringComparison.CurrentCultureIgnoreCase));
        return byProcess;
    }

    public void Toggle() => IsExpanded = !IsExpanded;

    partial void OnPreferenceIndexChanged(int oldValue, int newValue)
    {
        if (_syncing || newValue < 0)
        {
            return;
        }
        _ = _owner.SetPreferenceAsync(Path, (AppPreference)newValue, () => Restore((AppPreference)oldValue));
    }

    partial void OnCanExpandChanged(bool value) => OnPropertyChanged(nameof(ChevronVisibility));

    /// <summary>展开前先准备好进程行（这个通知在 IsExpanded 的通知之前，行据此量出展开后的高度）。</summary>
    partial void OnIsExpandedChanging(bool value)
    {
        if (value)
        {
            _everExpanded = true;
            SyncProcesses();
        }
    }

    partial void OnIsExpandedChanged(bool value) => OnPropertyChanged(nameof(ExpandLabel));

    /// <summary>偏好下拉框打开期间暂停列表刷新。</summary>
    internal void HoldRefresh(bool hold) => _owner.Hold(hold);

    /// <summary>重新读取列表时同步偏好、调度状态、占用和进程（不发请求）。</summary>
    internal void Update(AppEntry entry)
    {
        Restore(entry.Preference);
        Detail = AppsText.Detail(entry);
        CpuPct = entry.CpuPct;
        MemoryBytes = entry.MemoryBytes;
        Cpu = entry.Running ? AppsText.Cpu(entry.CpuPct) : "";
        Memory = entry.Running ? AppsText.Memory(entry.MemoryBytes) : "";
        _processes = entry.Processes;
        CanExpand = _processes.Count > 1;
        SyncProcesses();
    }

    /// <summary>设置显示的偏好但不发请求（服务拒绝时回到原来的选项）。</summary>
    private void Restore(AppPreference value)
    {
        _syncing = true;
        PreferenceIndex = (int)value;
        _syncing = false;
    }

    /// <summary>展开过的程序与最新的进程列表对齐（已有的行原地更新）。没展开过的不生成，省内存。</summary>
    private void SyncProcesses()
    {
        if (!_everExpanded)
        {
            return;
        }
        var existing = Processes.FirstByKey(p => p.Key);
        var rows = _processes.Select(p =>
        {
            if (existing.TryGetValue(ProcessItem.KeyOf(p), out var row))
            {
                row.Update(p);
                return row;
            }
            return new ProcessItem(_owner, p);
        }).ToList();
        CollectionSync.Sync(Processes, rows, p => p.Key);
    }
}
