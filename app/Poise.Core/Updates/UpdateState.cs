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
    Downloading,
    /// <summary>安装程序已经启动，界面随后退出。</summary>
    Installing,
    Failed,
}

public enum UpdateError
{
    /// <summary>连不上 GitHub 或下载中断。</summary>
    Network,
    /// <summary>下载的文件大小或校验值不对。</summary>
    Verification,
    /// <summary>安装程序没有启动（例如被安全软件拦截）。</summary>
    Launch,
}

/// <summary>
/// 更新的当前状态。<see cref="Release"/> 是已知的新版本：检查失败、下载失败后仍然保留，可以重试。
/// </summary>
public sealed record UpdateState(
    UpdatePhase Phase,
    ReleaseInfo? Release = null,
    double Progress = 0,
    UpdateError? Error = null)
{
    public bool IsBusy => Phase is UpdatePhase.Checking or UpdatePhase.Downloading or UpdatePhase.Installing;

    public bool CanCheck => Phase != UpdatePhase.Unavailable && !IsBusy;

    public bool CanInstall => Release is not null && !IsBusy;
}
