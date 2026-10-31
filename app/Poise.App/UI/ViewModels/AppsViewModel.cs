using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using Poise.Core.Ipc;
using Poise.Core.Presentation;
using Poise.App.UI.Services;
using Poise.Core.Collections;
using Poise.Core.Monitoring;
using Poise.Core.Settings;

namespace Poise.App.UI.ViewModels;

/// <summary>
/// 程序偏好页：智能调度开关；三栏可搜索的列表——正在运行的用户程序、
/// 系统（后台杂活与不参与调度的程序，只供查看）、未运行（设置过类别或运行过的程序）。
/// 正在运行的程序像任务管理器一样显示合计的 CPU 和内存，可展开到各个进程。
/// 前两栏按名称（拼音首字母与英文混排）、CPU 或内存排序，正序或倒序（规则见 <see cref="AppSortState"/>），
/// 选择保存在界面设置里；"未运行"总是按名称。偏好下拉框打开期间暂停刷新（<see cref="Hold"/>）。
/// 页面显示期间（<see cref="Start"/> 到 <see cref="Stop"/>）每 <see cref="LiveInterval"/> 重新读取一次
/// （主窗口被挡住时停下，见 <see cref="Poller"/>），调度状态跟随前台切换更新。
/// 定期读取与修改（设置类别、开关智能调度）经过 <see cref="RefreshGate"/>：修改之前发出的读取结果不显示，
/// 下拉框不会先跳回旧的类别。
/// </summary>
public sealed partial class AppsViewModel(AppHost host) : ObservableObject
{
    private static readonly TimeSpan LiveInterval = TimeSpan.FromSeconds(2);

    private readonly Poller _poller = new(host, LiveInterval);
    private readonly RefreshGate _gate = new();
    private List<AppItem> _all = [];
    /// <summary>按服务的状态同步开关时不发请求。</summary>
    private bool _syncing;
    private bool _wired;

    public ResettableCollection<AppItem> Running { get; } = [];

    public ResettableCollection<AppItem> System { get; } = [];

    public ResettableCollection<AppItem> NotRunning { get; } = [];

    [ObservableProperty]
    public partial string Query { get; set; } = "";

    /// <summary>"正在运行""系统"两栏的排序（列与方向），保存在界面设置里，下次打开沿用。</summary>
    [ObservableProperty]
    public partial AppSortState Sort { get; set; } = new(host.Settings.AppsSort, host.Settings.AppsSortDescending);

    /// <summary>第一次读取完成前为真。</summary>
    [ObservableProperty]
    public partial bool IsLoading { get; set; } = true;

    [ObservableProperty]
    public partial bool HasRunning { get; set; }

    [ObservableProperty]
    public partial bool HasSystem { get; set; }

    [ObservableProperty]
    public partial bool HasNotRunning { get; set; }

    /// <summary>读取完成后没有任何可显示的程序（或搜索没有结果）。</summary>
    [ObservableProperty]
    public partial bool IsEmpty { get; set; }

    /// <summary>智能调度开关（服务的设置）。用户切换时发请求，服务状态变化时跟着同步。</summary>
    [ObservableProperty]
    public partial bool SmartSchedule { get; set; } = true;

    /// <summary>出错时的提示（显示在页面顶部）。</summary>
    public event Action<string>? Error;

    /// <summary>出错之后又读成功了。</summary>
    public event Action? Recovered;

    /// <summary>页面显示时调用：读取列表，之后定期刷新。</summary>
    public void Start()
    {
        if (!_wired)
        {
            _wired = true;
            _poller.Error += message =>
            {
                // 读不到也不再一直转圈（和 0.7.0 一样），顶部提示原因；服务恢复后照常显示。
                IsLoading = false;
                Error?.Invoke(message);
            };
            _poller.Recovered += () => Recovered?.Invoke();
        }
        host.Status.Changed += SyncSmartSchedule;
        SyncSmartSchedule();
        _poller.Start(() => _gate.TickAsync(QueryAsync));
    }

    /// <summary>偏好下拉框打开时暂停定期刷新（重新排序会移动甚至重建它所在的行），关闭后恢复。</summary>
    public void Hold(bool holding) => _poller.Paused = holding;

    /// <summary>页面隐藏时调用：停止刷新。</summary>
    public void Stop()
    {
        _poller.Stop();
        host.Status.Changed -= SyncSmartSchedule;
    }

    partial void OnQueryChanged(string value) => ApplyFilter();

    partial void OnSortChanged(AppSortState value)
    {
        host.Settings.AppsSort = value.Column;
        host.Settings.AppsSortDescending = value.Descending;
        host.Settings.Save();
        ApplyFilter();
    }

    /// <summary>点了列名：换到这一列（按它的常用方向），或在同一列上反转方向。</summary>
    public void SortBy(AppSort column) => Sort = Sort.Click(column);

    partial void OnSmartScheduleChanged(bool value)
    {
        if (!_syncing)
        {
            _ = SetSmartScheduleAsync(value);
        }
    }

    private void SyncSmartSchedule()
    {
        _syncing = true;
        SmartSchedule = host.Status.Latest?.SmartSchedule ?? true;
        _syncing = false;
    }

    /// <summary>读取列表（失败时抛出，由调用方处理）；修改期间读到的不显示，修改完再读一次。</summary>
    private Task FetchAsync() => _gate.RefreshAsync(QueryAsync);

    private async Task<Action> QueryAsync()
    {
        var entries = await host.Api.ListAppsAsync();
        return () => Apply(entries);
    }

    private void Apply(List<AppEntry> entries)
    {
        // 重新读取时沿用已有的行（只更新偏好和状态），列表不重新入场，图标也不用重新加载。
        var previous = _all.GroupBy(i => (i.Key, i.Running)).ToDictionary(g => g.Key, g => g.First());
        _all = entries.Select(e =>
        {
            if (previous.TryGetValue((AppItem.KeyOf(e), e.Running), out var item))
            {
                item.Update(e);
                return item;
            }
            return new AppItem(this, e);
        }).ToList();
        IsLoading = false;
        ApplyFilter();
    }

    /// <summary>搜索匹配程序名或其中任一进程名；只有进程名匹配时展开该程序，便于看到是哪个进程。</summary>
    private void ApplyFilter()
    {
        var q = Query.Trim();
        var visible = new List<AppItem>();
        foreach (var item in _all)
        {
            if (item.Matches(q, out var byProcess))
            {
                visible.Add(item);
                if (byProcess)
                {
                    item.IsExpanded = true;
                }
            }
        }
        Replace(Running, Settled(Running, Ordered(visible.Where(a => a.Kind == AppKind.App && a.Running))));
        Replace(System, Settled(System, Ordered(visible.Where(a => a.Kind != AppKind.App))));
        Replace(NotRunning, visible.Where(a => a.Kind == AppKind.App && !a.Running).OrderBy(a => a.Name, NameOrder.Instance));
        UpdateSections();
    }

    private IEnumerable<AppItem> Ordered(IEnumerable<AppItem> items) =>
        // 按显示的精度比较（CPU 到 0.1%）：看起来相同的数值按名称排，不因看不见的差别来回跳。
        AppsText.Order(items, Sort, a => a.Name, a => MathF.Round(a.CpuPct, 1), a => a.MemoryBytes);

    /// <summary>按占用排序时，相近的行保持原来的位置（见 <see cref="AppsText.Settle"/>）。</summary>
    private IEnumerable<AppItem> Settled(ObservableCollection<AppItem> current, IEnumerable<AppItem> ordered)
    {
        var sorted = ordered.ToList();
        return Sort.Column switch
        {
            AppSort.Cpu => AppsText.Settle(current, sorted, a => a.Key, a => a.CpuPct, Sort.Descending, slack: 0.5),
            AppSort.Memory => AppsText.Settle(current, sorted, a => a.Key, a => a.MemoryBytes, Sort.Descending, slack: 16 * 1024 * 1024),
            _ => sorted,
        };
    }

    private void UpdateSections()
    {
        HasRunning = Running.Count > 0;
        HasSystem = System.Count > 0;
        HasNotRunning = NotRunning.Count > 0;
        IsEmpty = !IsLoading && !HasRunning && !HasSystem && !HasNotRunning;
    }

    /// <summary>原地调整（搜索时只有增减的行有动画，其余不动）。</summary>
    private static void Replace(ObservableCollection<AppItem> target, IEnumerable<AppItem> items) =>
        CollectionSync.Sync(target, items.ToList(), i => i.Key);

    /// <summary>为不在列表里的程序（例如没在运行）设置偏好，成功后重新读取列表。</summary>
    public Task<bool> AddAsync(string path, AppPreference preference) => TryAsync(async () =>
    {
        await _gate.ChangeAsync(() => host.Api.SetAppPreferenceAsync(path, preference));
        await FetchAsync();
    });

    /// <summary>开关智能调度：先按预期更新状态（界面立即响应），再重新读取列表里的调度状态。</summary>
    private Task SetSmartScheduleAsync(bool on) => TryAsync(async () =>
    {
        await _gate.ChangeAsync(() =>
            host.Status.ChangeAsync(() => host.Api.SetSmartScheduleAsync(on), s => ExpectedStatus.WithSmartSchedule(s, on)));
        await FetchAsync();
    });

    /// <summary>
    /// 设置一个程序（按路径）的类别。立即生效，顺便刷新列表；设置失败时调用 <paramref name="revert"/> 回到原来的选项。
    /// 设置成功、只是之后的刷新失败时不回退（类别已经改了），下一次刷新会补上。
    /// </summary>
    internal async Task SetPreferenceAsync(string path, AppPreference value, Action revert)
    {
        if (!await TryAsync(() => _gate.ChangeAsync(() => host.Api.SetAppPreferenceAsync(path, value))))
        {
            revert();
            return;
        }
        await TryAsync(FetchAsync);
    }

    /// <summary>执行一个服务请求；服务不可用或拒绝时在页面顶部提示，返回是否成功。</summary>
    private async Task<bool> TryAsync(Func<Task> action)
    {
        try
        {
            await action();
            return true;
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            Error?.Invoke(ErrorText.For(e));
            return false;
        }
    }
}
