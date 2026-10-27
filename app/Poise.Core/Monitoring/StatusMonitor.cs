using Poise.Core.Diagnostics;
using Poise.Core.Ipc;

namespace Poise.Core.Monitoring;

/// <summary>
/// 定期向服务查询状态：窗口或托盘面板显示时每秒一次，否则每 10 秒一次。
/// 结果在创建它的线程（界面线程）上通过 <see cref="Changed"/> 通知。
/// </summary>
public sealed class StatusMonitor : IDisposable
{
    private static readonly TimeSpan Visible = TimeSpan.FromSeconds(1);
    private static readonly TimeSpan Hidden = TimeSpan.FromSeconds(10);

    private readonly PoiseApi _api;
    private readonly SynchronizationContext _ui;
    private readonly Timer _timer;
    private bool _busy;

    /// <summary>必须在界面线程上创建（需要它的 <see cref="SynchronizationContext"/>）。</summary>
    public StatusMonitor(PoiseApi api)
    {
        _api = api;
        _ui = SynchronizationContext.Current
              ?? throw new InvalidOperationException("StatusMonitor 必须在界面线程上创建");
        _timer = new Timer(_ => _ui.Post(_ => _ = RefreshAsync(), null), null, TimeSpan.Zero, Hidden);
    }

    public Status? Latest { get; private set; }

    public bool Connected { get; private set; }

    /// <summary>在界面线程上触发。</summary>
    public event Action? Changed;

    public bool Fast
    {
        set
        {
            var interval = value ? Visible : Hidden;
            _timer.Change(interval, interval);
        }
    }

    public async Task RefreshAsync()
    {
        if (_busy)
        {
            return;
        }
        _busy = true;
        try
        {
            Latest = await _api.GetStatusAsync();
            Connected = true;
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            Connected = false;
        }
        finally
        {
            _busy = false;
        }
        foreach (var handler in Changed?.GetInvocationList() ?? [])
        {
            try
            {
                ((Action)handler)();
            }
            catch (Exception e)
            {
                // 某个界面处理状态时出错：记下来，不影响其他订阅者和后续刷新。
                AppLog.Error("处理状态更新时出错", e);
            }
        }
    }

    public void Dispose() => _timer.Dispose();
}
