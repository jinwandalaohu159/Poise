using System.Diagnostics;
using Microsoft.UI.Dispatching;
using Poise.Core.Ipc;
using Poise.Core.Monitoring;

namespace Poise.App.UI.Services;

/// <summary>
/// 走势数据的唯一来源，主页与卡顿诊断的走势卡共用（两者不会同时显示）。
/// 有走势卡在显示（<see cref="Attach"/> 到 <see cref="Detach"/>）且主窗口可见时，每 <see cref="Interval"/> 读取一次，否则停止。
/// 保留最近一次的结果：切换页面时新的走势卡直接接着画，不从空白开始。
/// </summary>
public sealed class TrendFeed : IDisposable
{
    private static readonly TimeSpan Interval = TimeSpan.FromSeconds(2);
    private static readonly Stopwatch Clock = Stopwatch.StartNew();

    private readonly PoiseApi _api;
    private readonly UiPresence _presence;
    private readonly DispatcherQueueTimer _timer;
    private int _viewers;
    private bool _busy;

    public TrendFeed(PoiseApi api, UiPresence presence, DispatcherQueue dispatcher)
    {
        _api = api;
        _presence = presence;
        _timer = dispatcher.CreateTimer();
        _timer.Interval = Interval;
        _timer.Tick += async (_, _) => await RefreshAsync();
        _presence.MainVisibilityChanged += UpdateTimer;
    }

    /// <summary>本地时钟（单调），走势上每个点的时刻都以它为准。</summary>
    public static TimeSpan Now => Clock.Elapsed;

    public TrendSeries Latest { get; private set; } = TrendSeries.Empty;

    /// <summary>在界面线程上触发。</summary>
    public event Action? Updated;

    /// <summary>走势卡显示时调用。</summary>
    public void Attach()
    {
        _viewers++;
        UpdateTimer();
    }

    /// <summary>走势卡隐藏时调用。</summary>
    public void Detach()
    {
        _viewers = Math.Max(0, _viewers - 1);
        UpdateTimer();
    }

    private void UpdateTimer()
    {
        if (_viewers > 0 && _presence.MainVisible)
        {
            if (!_timer.IsRunning)
            {
                _timer.Start();
                _ = RefreshAsync();
            }
        }
        else
        {
            _timer.Stop();
        }
    }

    private async Task RefreshAsync()
    {
        if (_busy)
        {
            return;
        }
        _busy = true;
        try
        {
            var trend = await _api.GetTrendAsync();
            Latest = TrendSeries.From(trend, Now);
            Updated?.Invoke();
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            // 服务没在运行：保留上一次的走势，主页的状态区会显示"未连接"。
        }
        finally
        {
            _busy = false;
        }
    }

    public void Dispose()
    {
        _timer.Stop();
        _presence.MainVisibilityChanged -= UpdateTimer;
    }
}
