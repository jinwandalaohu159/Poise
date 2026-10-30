using Microsoft.UI.Dispatching;
using Poise.Core.Ipc;
using Poise.Core.Presentation;

namespace Poise.App.UI.Services;

/// <summary>
/// 页面显示期间定期读取：<see cref="Start"/> 时立即读一次，之后每隔一段时间再读；主窗口被挡住或最小化时跳过
/// （见 <see cref="UiPresence"/>）；上一次还没回来（服务忙）时跳过这一次，不排队。读取失败时触发 <see cref="Error"/>。
/// 只在界面线程上使用。
/// </summary>
public sealed class Poller(AppHost host, TimeSpan interval)
{
    private DispatcherQueueTimer? _timer;
    private Func<Task>? _load;
    private bool _loading;

    /// <summary>读取失败时给用户看的原因。</summary>
    public event Action<string>? Error;

    public void Start(Func<Task> load)
    {
        _load = load;
        if (_timer is null)
        {
            _timer = DispatcherQueue.GetForCurrentThread().CreateTimer();
            _timer.Interval = interval;
            _timer.Tick += async (_, _) =>
            {
                if (host.Presence.MainVisible)
                {
                    await RunAsync();
                }
            };
        }
        _timer.Start();
        _ = RunAsync();
    }

    public void Stop() => _timer?.Stop();

    private async Task RunAsync()
    {
        if (_loading || _load is null)
        {
            return;
        }
        _loading = true;
        try
        {
            await _load();
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            Error?.Invoke(ErrorText.For(e));
        }
        finally
        {
            _loading = false;
        }
    }
}
