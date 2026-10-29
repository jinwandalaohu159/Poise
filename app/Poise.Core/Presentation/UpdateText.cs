using Poise.Core.Localization;
using Poise.Core.Updates;

namespace Poise.Core.Presentation;

/// <summary>设置页"更新"一节的文字。</summary>
public static class UpdateText
{
    /// <summary>更新说明只显示开头几行，完整内容在发布页里。</summary>
    private const int NoteLines = 3;
    private const int NoteChars = 240;

    /// <summary>"Poise 0.2.1"；开发版本没有版本号。</summary>
    public static string VersionLine(Version? installed) =>
        installed is null ? Loc.Get("update.dev_build") : $"Poise {installed.ToString(3)}";

    public static string Status(UpdateState state) => state.Phase switch
    {
        UpdatePhase.Unavailable => Loc.Get("update.unavailable"),
        UpdatePhase.Idle => Loc.Get("update.idle"),
        UpdatePhase.Checking => Loc.Get("update.checking"),
        UpdatePhase.UpToDate => Loc.Get("update.up_to_date"),
        UpdatePhase.Available => Loc.Format("update.available", state.Release!.Version.ToString(3)),
        UpdatePhase.Starting => Loc.Get("update.installing"),
        UpdatePhase.Failed => Loc.Get(state.Error == UpdateError.Launch ? "update.failed.launch" : "update.failed.network"),
        _ => "",
    };

    /// <summary>新版本说明的开头几行（去掉 Markdown 的标题和列表符号）；没有新版本时为空。</summary>
    public static string Notes(UpdateState state)
    {
        if (state.Release is not { } release || state.Phase is UpdatePhase.Checking)
        {
            return "";
        }
        var lines = release.Notes
            .Split('\n')
            .Select(l => l.Trim().TrimStart('#', '-', '*', ' ').Replace("**", ""))
            .Where(l => l.Length > 0)
            .Take(NoteLines);
        var text = string.Join("\n", lines);
        return text.Length > NoteChars ? text[..NoteChars].TrimEnd() + "…" : text;
    }
}
