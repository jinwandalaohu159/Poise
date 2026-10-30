using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using Microsoft.UI.Xaml;
using Poise.App.UI.Services;
using Poise.Core.Presentation;

namespace Poise.App.UI.ViewModels;

/// <summary>
/// 使用记录：显示期间（<see cref="Start"/> 到 <see cref="Stop"/>）每 <see cref="Interval"/> 读一次——一段结束才会多一条，
/// 条数和最新一条都没变时不动列表。服务由日志系统直接回答，不等调度引擎。
/// </summary>
public sealed partial class RecordsViewModel(AppHost host) : ObservableObject
{
    private static readonly TimeSpan Interval = TimeSpan.FromSeconds(20);

    private readonly Poller _poller = new(host, Interval);
    /// <summary>上次显示的记录：条数和最新一条的开始时间。</summary>
    private (int Count, ulong? Newest) _shown = (-1, null);

    /// <summary>最新的在前。</summary>
    public ObservableCollection<RecordView> Records { get; } = [];

    public Visibility EmptyVisibility => Records.Count == 0 ? Visibility.Visible : Visibility.Collapsed;

    /// <summary>出错时的提示（显示在页面顶部）。</summary>
    public event Action<string>? Error
    {
        add => _poller.Error += value;
        remove => _poller.Error -= value;
    }

    public void Start() => _poller.Start(LoadAsync);

    public void Stop() => _poller.Stop();

    private async Task LoadAsync()
    {
        var list = await host.Api.GetRecordsAsync();
        var shown = (list.Count, list.FirstOrDefault()?.StartedUnixMs);
        if (shown == _shown)
        {
            return;
        }
        _shown = shown;
        var machine = Machine.From(host.Status.Latest);
        var now = DateTimeOffset.Now;
        Records.Clear();
        foreach (var r in list)
        {
            Records.Add(RecordText.Build(r, machine, now));
        }
        OnPropertyChanged(nameof(EmptyVisibility));
    }
}
