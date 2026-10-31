using Poise.Core.Diagnostics;
using Poise.Core.Ipc;
using Poise.Core.Localization;

namespace Poise.Core.Presentation;

/// <summary>
/// 失败时给用户看的一句说明（按界面语言）。服务给的原文只写进日志：和上一条一样的不重复写
/// （服务没运行时每次刷新都失败，日志不该每隔一两秒多一行）。
/// </summary>
public static class ErrorText
{
    private static string? _lastLogged;

    /// <summary>记下原因，返回给用户看的说明。</summary>
    public static string For(Exception e)
    {
        Log(e);
        return Describe(e);
    }

    /// <summary>给用户看的说明（不记日志）。</summary>
    public static string Describe(Exception e) => Loc.Get(Key(e));

    /// <summary>把原因写进日志；和上一条相同时不写。</summary>
    public static void Log(Exception e)
    {
        var line = $"操作未完成：{e.GetType().Name}: {e.Message}";
        if (Interlocked.Exchange(ref _lastLogged, line) != line)
        {
            AppLog.Warn(line);
        }
    }

    internal static string Key(Exception e) => e switch
    {
        ServiceUnavailableException => "error.unavailable",
        ServiceErrorException { Code: ErrorCode.ObserveOnly } => "error.observe_only",
        ServiceErrorException { Code: ErrorCode.ProcessGone } => "error.process_gone",
        ServiceErrorException { Code: ErrorCode.InvalidPath } => "error.invalid_path",
        ServiceErrorException { Code: ErrorCode.NotReady } => "error.not_ready",
        ServiceErrorException { Code: ErrorCode.NotFound } => "error.not_found",
        ServiceErrorException { Code: ErrorCode.Failed } => "error.failed",
        _ => "error.generic",
    };
}
