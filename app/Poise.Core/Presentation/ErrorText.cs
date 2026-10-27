using Poise.Core.Diagnostics;
using Poise.Core.Ipc;
using Poise.Core.Localization;

namespace Poise.Core.Presentation;

/// <summary>失败时给用户看的一句说明（按界面语言）。服务给的原文只写进日志。</summary>
public static class ErrorText
{
    public static string For(Exception e)
    {
        AppLog.Warn($"操作未完成：{e.GetType().Name}: {e.Message}");
        return Loc.Get(Key(e));
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
