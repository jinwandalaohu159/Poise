using System.Collections.ObjectModel;
using System.Diagnostics;
using CommunityToolkit.Mvvm.ComponentModel;
using Microsoft.UI.Xaml;
using Poise.App.UI.Services;
using Poise.Core.Presentation;

namespace Poise.App.UI.ViewModels;

/// <summary>
/// 日志：第一次取最新的一批，之后每 <see cref="LiveInterval"/> 只取比已有的更新的几条，最新的在上面，可按类别筛选。
/// 只在日志显示期间（<see cref="Start"/> 到 <see cref="Stop"/>）读取，主窗口被挡住或最小化时不读。
/// 服务由日志系统直接回答，不等调度引擎。第一次（几百条）和换筛选时整张列表一次换上，不逐条插入
/// （每插一条列表都要通知、重新排版一次）。
/// </summary>
public sealed partial class EventLogViewModel(AppHost host) : ObservableObject
{
    private static readonly TimeSpan LiveInterval = TimeSpan.FromSeconds(2);
    /// <summary>界面里最多留多少条（服务那边留得更多，见 events.jsonl）。</summary>
    private const int MaxRows = 1000;

    /// <summary>新来的多于这么多条时整张列表换上，不逐条插入。</summary>
    private const int BulkRows = 20;

    /// <summary>全部日志，从早到晚（新的追加在后面）。</summary>
    private readonly List<EventRow> _all = [];
    private readonly Poller _poller = new(host, LiveInterval);
    private ulong _last;
    private string? _logDir;

    /// <summary>筛选后的日志，最新的在前。</summary>
    [ObservableProperty]
    public partial ObservableCollection<EventRow> Events { get; private set; } = [];

    public IReadOnlyList<EventFilter> Filters { get; } = EventText.Filters();

    [ObservableProperty]
    public partial EventFilter? Filter { get; set; }

    public Visibility EmptyVisibility => Events.Count == 0 ? Visibility.Visible : Visibility.Collapsed;

    public bool CanOpenFolder => _logDir is not null;

    /// <summary>出错时的提示（显示在页面顶部）。</summary>
    public event Action<string>? Error
    {
        add => _poller.Error += value;
        remove => _poller.Error -= value;
    }

    /// <summary>出错之后又读成功了。</summary>
    public event Action? Recovered
    {
        add => _poller.Recovered += value;
        remove => _poller.Recovered -= value;
    }

    public void Start()
    {
        Filter ??= Filters[0];
        _poller.Start(LoadAsync);
    }

    public void Stop() => _poller.Stop();

    /// <summary>用资源管理器打开详细日志（poise.log）所在的文件夹。</summary>
    public void OpenFolder()
    {
        if (_logDir is { } dir)
        {
            Process.Start(new ProcessStartInfo("explorer.exe", $"\"{dir}\"") { UseShellExecute = true });
        }
    }

    partial void OnFilterChanged(EventFilter? value) => Rebuild();

    /// <summary>按筛选整张换上（最新的在前）。</summary>
    private void Rebuild()
    {
        Events = new ObservableCollection<EventRow>(Enumerable.Reverse(_all).Where(Passes));
        OnPropertyChanged(nameof(EmptyVisibility));
    }

    private async Task LoadAsync()
    {
        var page = await host.Api.GetEventsAsync(_last);
        if (_logDir is null)
        {
            _logDir = page.LogDir;
            OnPropertyChanged(nameof(CanOpenFolder));
        }
        if (page.Entries.Count == 0)
        {
            return;
        }
        var machine = Machine.From(host.Status.Latest);
        var now = DateTimeOffset.Now;
        // 服务给的是从早到晚。
        var rows = page.Entries.Select(e => EventText.Row(e, machine, now)).ToList();
        var newest = page.Entries.Max(e => e.Id);
        // 都不比已有的新：服务的日志重新编号了（日志文件被删掉、重新开始），给的是最新的一批，整张换上。
        if (newest <= _last)
        {
            _all.Clear();
        }
        _last = newest;
        _all.AddRange(rows);
        var dropped = Math.Max(0, _all.Count - MaxRows);
        if (_all.Count == rows.Count || rows.Count > BulkRows)
        {
            _all.RemoveRange(0, dropped);
            Rebuild();
            return;
        }
        // 少量新的：逐条放到最上面，最早的从底下移走。
        foreach (var row in rows.Where(Passes))
        {
            Events.Insert(0, row);
        }
        foreach (var oldest in _all.Take(dropped))
        {
            if (Events.Count > 0 && ReferenceEquals(Events[^1], oldest))
            {
                Events.RemoveAt(Events.Count - 1);
            }
        }
        _all.RemoveRange(0, dropped);
        OnPropertyChanged(nameof(EmptyVisibility));
    }

    private bool Passes(EventRow row) => Filter?.Category is not { } category || row.Category == category;
}
