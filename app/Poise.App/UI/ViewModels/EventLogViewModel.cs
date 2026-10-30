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
/// 服务由日志系统直接回答，不等调度引擎。
/// </summary>
public sealed partial class EventLogViewModel(AppHost host) : ObservableObject
{
    private static readonly TimeSpan LiveInterval = TimeSpan.FromSeconds(2);
    /// <summary>界面里最多留多少条（服务那边留得更多，见 events.jsonl）。</summary>
    private const int MaxRows = 1000;

    /// <summary>全部日志，最新的在前。</summary>
    private readonly List<EventRow> _all = [];
    private readonly Poller _poller = new(host, LiveInterval);
    private ulong _last;
    private string? _logDir;

    /// <summary>筛选后的日志，最新的在前。</summary>
    public ObservableCollection<EventRow> Events { get; } = [];

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

    partial void OnFilterChanged(EventFilter? value)
    {
        Events.Clear();
        foreach (var row in _all.Where(Passes))
        {
            Events.Add(row);
        }
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
        // 服务给的是从早到晚：逐条放到最上面。
        foreach (var entry in page.Entries)
        {
            var row = EventText.Row(entry, machine, now);
            _all.Insert(0, row);
            if (Passes(row))
            {
                Events.Insert(0, row);
            }
            _last = Math.Max(_last, entry.Id);
        }
        while (_all.Count > MaxRows)
        {
            var oldest = _all[^1];
            _all.RemoveAt(_all.Count - 1);
            if (Events.Count > 0 && ReferenceEquals(Events[^1], oldest))
            {
                Events.RemoveAt(Events.Count - 1);
            }
        }
        OnPropertyChanged(nameof(EmptyVisibility));
    }

    private bool Passes(EventRow row) => Filter?.Category is not { } category || row.Category == category;
}
