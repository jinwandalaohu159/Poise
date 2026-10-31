using Microsoft.UI.Dispatching;
using Poise.Core.Ipc;
using Poise.Core.Monitoring;

namespace Poise.App.UI.Services;

/// <summary>
/// 界面可见性的唯一入口：
/// 主窗口或托盘面板显示时才刷新状态；主窗口显示期间定期告诉服务"界面在显示"（服务据此读取显卡）；
/// 两者都不显示时触发 <see cref="AllHidden"/>，界面据此在闲置一段时间后退出、释放内存。
/// 走势图只在主窗口显示时读取（<see cref="MainVisibilityChanged"/>）。
/// </summary>
public sealed class UiPresence : IDisposable
{
    /// <summary>服务认为"界面在显示"的有效期是 30 秒，这里每 10 秒重复一次。</summary>
    private static readonly TimeSpan VisibleHeartbeat = TimeSpan.FromSeconds(10);

    private readonly StatusMonitor _status;
    private readonly PoiseApi _api;
    private readonly DispatcherQueueTimer _heartbeat;
    private bool _main;
    private bool _flyout;
    private bool _reporting;

    public UiPresence(StatusMonitor status, PoiseApi api, DispatcherQueue dispatcher)
    {
        _status = status;
        _api = api;
        _heartbeat = dispatcher.CreateTimer();
        _heartbeat.Interval = VisibleHeartbeat;
        _heartbeat.Tick += (_, _) => ReportMain();
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
        if (visible)
        {
            _heartbeat.Start();
        }
        else
        {
            _heartbeat.Stop();
        }
        Update(wasVisible);
        ReportMain();
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
        // 先通知显示出来了（连接立即重试，不等重连间隔），再查状态、上报。
        if (!wasVisible && AnyVisible)
        {
            AnyShown?.Invoke();
        }
        if (wasVisible != AnyVisible)
        {
            // 变成显示时立即查一次：面板一弹出就是最新的。
            _status.Watched = AnyVisible;
        }
        if (wasVisible && !AnyVisible)
        {
            AllHidden?.Invoke();
        }
    }

    private async void ReportMain()
    {
        // 上一次还没回来（服务忙）：不再排一次，下次心跳再报。
        if (_reporting)
        {
            return;
        }
        _reporting = true;
        try
        {
            await _api.SetUiVisibleAsync(_main);
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            // 服务没在运行：状态区会显示"未连接"。
        }
        finally
        {
            _reporting = false;
        }
    }

    public void Dispose() => _heartbeat.Stop();
}
