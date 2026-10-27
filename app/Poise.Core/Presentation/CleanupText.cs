using Poise.Core.Ipc;
using Poise.Core.Localization;

namespace Poise.Core.Presentation;

/// <summary>清理结果与清理记录的文案。</summary>
public static class CleanupText
{
    /// <summary>"一键清理"完成后的提示。</summary>
    public static Feedback Describe(CleanupResult r, bool dryRun)
    {
        var details = new List<string>();
        if (r.Deferred > 0)
        {
            details.Add(Loc.Format("clean.deferred", r.Deferred));
        }
        foreach (var a in r.Advisories)
        {
            switch (a.Kind)
            {
                case "heavy_app":
                    details.Add(Loc.Format("clean.heavy_app", a.Name));
                    break;
                case "restart_recommended":
                    details.Add(Loc.Get("clean.restart"));
                    break;
            }
        }
        if (r.Failed.Count > 0)
        {
            details.Add(Loc.Format("clean.failed", r.Failed.Count));
        }
        if (dryRun)
        {
            details.InsertRange(0, r.Handled);
            return new Feedback(FeedbackKind.Info, Loc.Get("clean.observe"), details);
        }
        var message = r.FreedBytes > 0
            ? Loc.Format("clean.done", Format.Bytes(r.FreedBytes))
            : r.Handled.Count > 0 ? Loc.Format("clean.done_count", r.Handled.Count) : Loc.Get("clean.nothing");
        return new Feedback(r.Handled.Count > 0 ? FeedbackKind.Success : FeedbackKind.Info, message, details);
    }

    /// <summary>自动清理一行的说明：做过清理时显示上次的结果，否则显示功能说明。</summary>
    public static string AutoCleanDetail(LastCleanup? last, DateTimeOffset now)
    {
        if (last is null)
        {
            return Loc.Get("home.auto_clean.desc");
        }
        var when = Format.When(DateTimeOffset.FromUnixTimeMilliseconds((long)last.AtUnixMs), now);
        return last.FreedBytes > 0
            ? Loc.Format("home.last_clean.freed", when, Format.Bytes(last.FreedBytes))
            : Loc.Format("home.last_clean.count", when, last.Handled);
    }
}
