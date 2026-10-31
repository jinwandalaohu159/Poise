using Poise.Core.Ipc;
using Poise.Core.Presentation;

namespace Poise.App.UI.ViewModels;

/// <summary>一键清理（主页、托盘面板、清理页共用）：执行并把结果换成提示文案。</summary>
internal static class CleanupCommand
{
    /// <summary>
    /// 清理要等它做完（重启服务可能十几秒）。不经过整体状态的修改通道（<see cref="Poise.Core.Monitoring.StatusMonitor.ChangeAsync"/>）：
    /// 那样这十几秒里整体状态的刷新全被丢掉，主页和托盘面板的数字停住不动；清理改的只是清理相关的数字，
    /// 做完刷新一次就对了。
    /// </summary>
    /// <param name="refresh">做完之后要刷新的：主页刷新整体状态，清理页还要刷新本页。</param>
    public static async Task<Feedback> RunAsync(AppHost host, Func<Task> refresh)
    {
        try
        {
            var result = await host.Api.CleanNowAsync();
            return CleanupText.Describe(result, host.Status.Latest?.DryRun == true);
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            return Feedback.Error(ErrorText.For(e));
        }
        finally
        {
            await refresh();
        }
    }
}
