using Microsoft.UI.Dispatching;
using Poise.Core.Updates;

namespace Poise.App.UI.Services;

/// <summary>
/// 界面运行期间定时检查更新：启动后稍等片刻检查一次（不和首屏加载抢资源），之后每小时一次。
/// 定时检查不显示"正在检查"，失败时也不提示；托盘代理另外在后台定时检查，界面关着时也能及时提醒。
/// </summary>
public sealed class UpdateSchedule : IDisposable
{
    private static readonly TimeSpan FirstCheck = TimeSpan.FromSeconds(10);
    private static readonly TimeSpan Interval = TimeSpan.FromHours(1);

    private readonly Updater _updater;
    private readonly DispatcherQueueTimer _timer;

    public UpdateSchedule(Updater updater, DispatcherQueue dispatcher)
    {
        _updater = updater;
        _timer = dispatcher.CreateTimer();
        _timer.Interval = FirstCheck;
        _timer.Tick += async (_, _) =>
        {
            _timer.Interval = Interval;
            await Guard.RunAsync(() => _updater.CheckAsync(quiet: true), "检查更新");
        };
        if (updater.State.Phase != UpdatePhase.Unavailable)
        {
            _timer.Start();
        }
    }

    public void Dispose() => _timer.Stop();
}
