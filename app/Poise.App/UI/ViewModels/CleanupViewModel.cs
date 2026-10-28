using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using Microsoft.UI.Xaml;
using Poise.App.UI.Services;
using Poise.Core.Collections;
using Poise.Core.Ipc;
using Poise.Core.Monitoring;
using Poise.Core.Presentation;

namespace Poise.App.UI.ViewModels;

/// <summary>
/// 清理页：顶部摘要 + 按程序列出的明细。页面显示期间每 3 秒刷新。
/// 刷新与修改经过 <see cref="RefreshGate"/>：修改期间的旧结果不显示，开关不会闪回。
/// </summary>
public sealed partial class CleanupViewModel : ObservableObject, IDisposable
{
    private static readonly TimeSpan RefreshEvery = TimeSpan.FromSeconds(3);
    /// <summary>后一行比前一行多出这个比例时才重新排序，避免占用相近的程序来回换位。</summary>
    private const double ReorderMargin = 1.1;

    private readonly AppHost _host;
    private readonly DispatcherTimer _timer = new() { Interval = RefreshEvery };
    private readonly RefreshGate _gate = new();

    public CleanupViewModel(AppHost host)
    {
        _host = host;
        // 主窗口最小化或被全屏程序挡住时不刷新（见 UiPresence）。
        _timer.Tick += async (_, _) =>
        {
            if (_host.Presence.MainVisible)
            {
                await RefreshAsync();
            }
        };
    }

    public ObservableCollection<ProcessRowItem> Attention { get; } = [];

    public ObservableCollection<ProcessRowItem> Others { get; } = [];

    /// <summary>最近一次读到的摘要。</summary>
    public CleanupPageView? Current { get; private set; }

    /// <summary>摘要更新（界面线程）。</summary>
    public event Action<CleanupPageView>? Updated;

    public event Action<string>? Error;

    public async Task StartAsync()
    {
        await RefreshAsync();
        _timer.Start();
    }

    public void Stop() => _timer.Stop();

    public Task RefreshAsync() => _gate.RefreshAsync(async () =>
    {
        try
        {
            var overview = await _host.Api.CleanupOverviewAsync();
            return () => Apply(CleanupPageText.Build(overview, DateTimeOffset.Now));
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            return () => Error?.Invoke(ErrorText.For(e));
        }
    });

    /// <summary>一键清理，完成后同时刷新整体状态和本页。</summary>
    public Task<Feedback> CleanAsync() => CleanupCommand.RunAsync(_host, Change);

    public Task SetAutoCleanAsync(bool on) => TryChange(() => _host.Api.SetAutoCleanAsync(on));

    public Task SnoozeRestartAsync() => TryChange(_host.Api.SnoozeRestartAdviceAsync);

    /// <summary>修改后同时刷新整体状态（主页、托盘）和本页。</summary>
    private async Task Change(Func<Task> change)
    {
        await _gate.ChangeAsync(() => _host.Status.ChangeAsync(change));
        await RefreshAsync();
    }

    private async Task TryChange(Func<Task> change)
    {
        try
        {
            await Change(change);
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            Error?.Invoke(ErrorText.For(e));
        }
    }

    private void Apply(CleanupPageView view)
    {
        Current = view;
        Reconcile(Attention, view.Attention);
        Reconcile(Others, view.Others);
        Updated?.Invoke(view);
        _ = LoadIconsAsync();
    }

    /// <summary>
    /// 原地更新每一行。行的集合没变时，顺序只在明显过时才调整（占用相近的程序不来回换位）；
    /// 有行增减时按新顺序做最少的插入、删除、移动，列表过渡动画只作用于变化的行。
    /// </summary>
    private static void Reconcile(ObservableCollection<ProcessRowItem> target, IReadOnlyList<ProcessRowView> rows)
    {
        var existing = target.ToDictionary(i => i.Pid);
        var items = rows.Select(r =>
        {
            if (existing.TryGetValue(r.Pid, out var item))
            {
                item.Update(r);
                return item;
            }
            return new ProcessRowItem(r);
        }).ToList();
        var sameRows = items.Count == target.Count && items.All(i => existing.ContainsKey(i.Pid));
        var stale = target.Zip(target.Skip(1)).Any(p => p.Second.MemoryBytes > p.First.MemoryBytes * ReorderMargin);
        if (sameRows && !stale)
        {
            return;
        }
        CollectionSync.Sync(target, items, i => i.Pid);
    }

    private async Task LoadIconsAsync()
    {
        // AppIcons 按路径缓存，刷新时不会重复读取。
        foreach (var item in Attention.Concat(Others).Where(i => i.Icon is null && i.Path is not null).ToList())
        {
            item.Icon = await AppIcons.LoadAsync(item.Path!);
        }
    }

    public void Dispose() => _timer.Stop();
}
