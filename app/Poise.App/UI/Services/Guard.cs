using Poise.Core.Diagnostics;

namespace Poise.App.UI.Services;

/// <summary>
/// 定时器、按钮这类 async void 回调用：里面漏出的异常不经过 <see cref="Microsoft.UI.Xaml.Application.UnhandledException"/>，
/// 会直接结束进程。在这里记录后吞掉（服务不可用这类预期的错误由各处自己处理）；进程已经不可靠的除外。
/// </summary>
internal static class Guard
{
    public static async Task RunAsync(Func<Task> action, string what)
    {
        try
        {
            await action();
        }
        catch (Exception e) when (Recoverable(e))
        {
            AppLog.Error($"{what}失败", e);
        }
    }

    /// <summary>可以记录后继续运行的异常（内存耗尽这类不行）。</summary>
    public static bool Recoverable(Exception e) =>
        e is not (OutOfMemoryException or InsufficientExecutionStackException);
}
