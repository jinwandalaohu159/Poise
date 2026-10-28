using Poise.Core.Ipc;
using Poise.Core.Presentation;

namespace Poise.App.UI.ViewModels;

/// <summary>一键清理（主页、托盘面板、清理页共用）：执行并把结果换成提示文案。</summary>
internal static class CleanupCommand
{
    /// <param name="change">执行修改的通道：主页走整体状态刷新，清理页还要同时刷新本页。</param>
    public static async Task<Feedback> RunAsync(AppHost host, Func<Func<Task>, Task> change)
    {
        try
        {
            CleanupResult result = null!;
            await change(async () => result = await host.Api.CleanNowAsync());
            return CleanupText.Describe(result, host.Status.Latest?.DryRun == true);
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            return Feedback.Error(ErrorText.For(e));
        }
    }
}
