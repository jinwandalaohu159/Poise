using Microsoft.UI.Dispatching;
using Poise.Core.Ipc;
using Poise.Core.Presentation;

namespace Poise.App.UI.Services;

/// <summary>
/// 页面显示期间定期读取：<see cref="Start"/> 时立即读一次，之后每隔一段时间再读。主窗口被挡住或最小化时
/// 定时器停下（见 <see cref="UiPresence"/>），重新显示时立即读一次再继续；上一次还没回来（服务忙）时跳过
/// 这一次，不排队；<see cref="Paused"/> 期间也跳过（例如下拉框打开着，刷新会重建它所在的行）。
/// 读取失败时触发 <see cref="Error"/>，之后第一次成功时触发 <see cref="Recovered"/>。只在界面线程上使用。
/// </summary>
public sealed class Poller(AppHost host, TimeSpan interval)
{
    private DispatcherQueueTimer? _timer;
    private Func<Task>? _load;
    private bool _loading;
    private bool _running;
    private bool _failing;

    /// <summary>读取失败时给用户看的原因。</summary>
    public event Action<string>? Error;

    /// <summary>失败之后又读成功了（页面据此收起错误提示）。</summary>
    public event Action? Recovered;

    /// <summary>暂停期间到时间也不读。</summary>
    public bool Paused { get; set; }

    public void Start(Func<Task> load)
    {
        _load = load;
        if (_timer is null)
        {
            _timer = DispatcherQueue.GetForCurrentThread().CreateTimer();
            _timer.Interval = interval;
            _timer.Tick += async (_, _) => await Guard.RunAsync(RunAsync, "定时刷新");
        }
        if (!_running)
        {
            _running = true;
            host.Presence.MainVisibilityChanged += OnVisibilityChanged;
        }
        OnVisibilityChanged();
    }

    public void Stop()
    {
        _timer?.Stop();
        if (_running)
        {
            _running = false;
            host.Presence.MainVisibilityChanged -= OnVisibilityChanged;
        }
    }

    /// <summary>立即读一次（例如用户刚改了什么）。</summary>
    public Task RefreshAsync() => RunAsync();

    private void OnVisibilityChanged()
    {
        if (host.Presence.MainVisible)
        {
            _timer?.Start();
            _ = RunAsync();
        }
        else
        {
            _timer?.Stop();
        }
    }

    private async Task RunAsync()
    {
        if (_loading || Paused || _load is null)
        {
            return;
        }
        _loading = true;
        try
        {
            await _load();
            if (_failing)
            {
                _failing = false;
                Recovered?.Invoke();
            }
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            _failing = true;
            Error?.Invoke(ErrorText.For(e));
        }
        finally
        {
            _loading = false;
        }
    }
}
