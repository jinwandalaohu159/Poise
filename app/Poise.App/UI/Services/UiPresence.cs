using Microsoft.UI.Dispatching;
using Poise.Core.Ipc;
using Poise.Core.Monitoring;

namespace Poise.App.UI.Services;

/// <summary>
/// 界面可见性的唯一入口：
/// 主窗口或托盘面板显示时才刷新状态，并定期告诉服务"界面在显示"（服务据此读取显卡、风扇，显示的那一刻
/// 立即采一轮）；没告诉成（服务刚启动、还没连上）时很快再试，不等心跳；
/// 两者都不显示时触发 <see cref="AllHidden"/>，界面据此在闲置一段时间后退出、释放内存。
/// 走势图只在主窗口显示时读取（<see cref="MainVisibilityChanged"/>）。
/// </summary>
public sealed class UiPresence : IDisposable
{
    /// <summary>服务认为"界面在显示"的有效期是 30 秒，这里每 10 秒重复一次。</summary>
    private static readonly TimeSpan VisibleHeartbeat = TimeSpan.FromSeconds(10);

    /// <summary>没告诉成服务时，多久再试一次。</summary>
    private static readonly TimeSpan ReportRetry = TimeSpan.FromSeconds(1);

    private readonly StatusMonitor _status;
    private readonly PoiseApi _api;
    private readonly DispatcherQueueTimer _heartbeat;
    private readonly DispatcherQueueTimer _retry;
    private bool _main;
    private bool _flyout;
    private bool _reporting;
    /// <summary>上报期间可见性又变了：报完再报一次最新的。</summary>
    private bool _changedWhileReporting;

    public UiPresence(StatusMonitor status, PoiseApi api, DispatcherQueue dispatcher)
    {
        _status = status;
        _api = api;
        _heartbeat = dispatcher.CreateTimer();
        _heartbeat.Interval = VisibleHeartbeat;
        _heartbeat.Tick += (_, _) => Report();
        _retry = dispatcher.CreateTimer();
        _retry.Interval = ReportRetry;
        _retry.IsRepeating = false;
        _retry.Tick += (_, _) => Report();
    }

    /// <summary>主窗口和托盘面板都不显示了。</summary>
    public event Action? AllHidden;

    /// <summary>主窗口或托盘面板显示出来了。</summary>
    public event Action? AnyShown;

    public bool AnyVisible => _main || _flyout;

    /// <summary>主窗口是否显示（走势图只在主窗口显示时读取）。</summary>
    public bool MainVisible => _main;

    /// <summary>主窗口显示或隐藏了。</summary>
    public event Action? MainVisibilityChanged;

    public void SetMainVisible(bool visible)
    {
        if (_main == visible)
        {
            return;
        }
        var wasVisible = AnyVisible;
        _main = visible;
        Update(wasVisible);
        MainVisibilityChanged?.Invoke();
    }

    public void SetFlyoutVisible(bool visible)
    {
        if (_flyout == visible)
        {
            return;
        }
        var wasVisible = AnyVisible;
        _flyout = visible;
        // 托盘面板显示的就是主页的数字：每秒更新。
        if (visible)
        {
            _status.NeedFast(this);
        }
        else
        {
            _status.ReleaseFast(this);
        }
        Update(wasVisible);
    }

    private void Update(bool wasVisible)
    {
        // 先通知显示出来了（连接立即重试，不等重连间隔），再上报、查状态。
        if (!wasVisible && AnyVisible)
        {
            AnyShown?.Invoke();
        }
        if (wasVisible != AnyVisible)
        {
            if (AnyVisible)
            {
                _heartbeat.Start();
            }
            else
            {
                _heartbeat.Stop();
                _retry.Stop();
            }
            Report();
            // 变成显示时立即查一次：面板一弹出就是最新的。
            _status.Watched = AnyVisible;
        }
        if (wasVisible && !AnyVisible)
        {
            AllHidden?.Invoke();
        }
    }

    /// <summary>告诉服务界面是不是在显示。上一次还没回来时，报完再报一次最新的（显示了又马上隐藏，
    /// 服务不会一直以为界面开着）。</summary>
    private async void Report()
    {
        if (_reporting)
        {
            _changedWhileReporting = true;
            return;
        }
        _reporting = true;
        try
        {
            do
            {
                _changedWhileReporting = false;
                await _api.SetUiVisibleAsync(AnyVisible);
            }
            while (_changedWhileReporting);
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            // 服务没在运行、还没连上：显示着就很快再试（状态区会显示"未连接"）。
            if (AnyVisible)
            {
                _retry.Start();
            }
        }
        finally
        {
            _reporting = false;
        }
    }

    public void Dispose()
    {
        _heartbeat.Stop();
        _retry.Stop();
    }
}
