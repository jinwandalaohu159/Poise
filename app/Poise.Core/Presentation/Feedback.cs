namespace Poise.Core.Presentation;

public enum FeedbackKind { Success, Info, Warning, Error }

/// <summary>提示里的数字从 0 递增到 <see cref="To"/>，每一帧用 <see cref="Text"/> 生成整句。</summary>
public sealed record CountUp(double To, Func<double, string> Text);

/// <summary>一次操作完成后给用户的一句话（显示在按钮下方的提示条里）。</summary>
public sealed record Feedback(
    FeedbackKind Kind,
    string Message,
    IReadOnlyList<string> Details,
    string? ActionLabel = null,
    Action? Action = null,
    CountUp? Count = null)
{
    public static Feedback Error(string message) => new(FeedbackKind.Error, message, []);
}
