namespace Poise.Core.Updates;

public enum UpdatePhase
{
    /// <summary>开发版本（没有 version.txt）：不检查更新。</summary>
    Unavailable,
    /// <summary>还没有检查过。</summary>
    Idle,
    Checking,
    UpToDate,
    /// <summary>有新版本，可以更新。</summary>
    Available,
    /// <summary>更新助手已经启动，界面随后退出。</summary>
    Starting,
    Failed,
}

public enum UpdateError
{
    /// <summary>连不上 GitHub。</summary>
    Network,
    /// <summary>更新助手没有启动（例如被安全软件拦截）。</summary>
    Launch,
}

/// <summary>
/// 更新的当前状态。<see cref="Release"/> 是已知的新版本：检查失败后仍然保留，可以重试。
/// </summary>
public sealed record UpdateState(
    UpdatePhase Phase,
    ReleaseInfo? Release = null,
    UpdateError? Error = null)
{
    public bool IsBusy => Phase is UpdatePhase.Checking or UpdatePhase.Starting;

    public bool CanCheck => Phase != UpdatePhase.Unavailable && !IsBusy;

    public bool CanInstall => Release is not null && !IsBusy;
}
