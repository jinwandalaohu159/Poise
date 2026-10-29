using System.ComponentModel;
using System.Diagnostics;
using Poise.Core.Diagnostics;
using Poise.Core.Localization;

namespace Poise.Core.Updates;

/// <summary>
/// 检查新版本，并在用户确认后交给更新助手。
/// <list type="bullet">
/// <item>只访问 GitHub（查询最新版本）；开发版本不联网。</item>
/// <item>同一时间只做一件事；方法从不抛异常，结果反映在 <see cref="State"/> 上并记日志。</item>
/// <item>在界面线程上调用，<see cref="Changed"/> 也在界面线程上触发。</item>
/// </list>
/// 下载、安装、重新启动 Poise 都由更新助手（poise-updater）完成，它有自己的进度窗口；
/// 它启动后触发 <see cref="UpdaterStarted"/>，界面应随即退出（安装程序要替换界面的文件）。
/// </summary>
public sealed class Updater : IDisposable
{
    private static readonly TimeSpan CheckTimeout = TimeSpan.FromSeconds(15);

    private readonly Version? _installed;
    private readonly UpdateFiles _files;
    private readonly HttpClient _http;
    private bool _working;

    public Updater(Version? installed, UpdateFiles files, HttpMessageHandler? handler = null)
    {
        _installed = installed;
        _files = files;
        _http = handler is null ? new HttpClient() : new HttpClient(handler);
        _http.Timeout = CheckTimeout;
        State = installed is null ? new UpdateState(UpdatePhase.Unavailable) : new UpdateState(UpdatePhase.Idle);
    }

    public UpdateState State { get; private set; }

    public event Action? Changed;

    public event Action? UpdaterStarted;

    /// <summary>
    /// 查询最新版本。<paramref name="quiet"/> 为真时（定时检查）不显示"正在检查"，失败也保持原状态，只记日志。
    /// </summary>
    public async Task CheckAsync(bool quiet = false)
    {
        if (_installed is null || _working)
        {
            return;
        }
        _working = true;
        var known = State.Release;
        try
        {
            if (!quiet)
            {
                Set(new UpdateState(UpdatePhase.Checking, known));
            }
            using var cancel = new CancellationTokenSource(CheckTimeout);
            var latest = await GitHubReleases.FetchLatestAsync(_http, _installed, cancel.Token);
            if (latest is not null && latest.Version > _installed)
            {
                if (latest.Version != known?.Version)
                {
                    AppLog.Info($"发现新版本 {latest.Tag}");
                }
                Set(new UpdateState(UpdatePhase.Available, latest));
            }
            else
            {
                // 已是最新：上次更新留下的安装程序不再需要。
                _files.DeleteInstallers();
                Set(new UpdateState(UpdatePhase.UpToDate));
            }
        }
        catch (Exception e) when (e is HttpRequestException or OperationCanceledException or IOException)
        {
            AppLog.Warn($"检查更新失败：{e.Message}");
            if (!quiet)
            {
                Set(new UpdateState(UpdatePhase.Failed, known, Error: UpdateError.Network));
            }
        }
        finally
        {
            _working = false;
        }
    }

    /// <summary>启动更新助手（它下载、校验、安装并重新启动 Poise）。只在已知有新版本时有效。</summary>
    public void StartUpdate()
    {
        if (State.Release is not { } release || _working)
        {
            return;
        }
        try
        {
            var start = new ProcessStartInfo(_files.PrepareUpdater()) { UseShellExecute = false };
            foreach (var arg in new[] { "--dir", _files.InstallDirectory, "--lang", Loc.Language })
            {
                start.ArgumentList.Add(arg);
            }
            Process.Start(start)?.Dispose();
            Set(new UpdateState(UpdatePhase.Starting, release));
            AppLog.Info($"已启动更新助手，更新到 {release.Tag}");
            UpdaterStarted?.Invoke();
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or Win32Exception or InvalidOperationException)
        {
            AppLog.Warn($"启动更新助手失败：{e.Message}");
            Set(new UpdateState(UpdatePhase.Failed, release, Error: UpdateError.Launch));
        }
    }

    private void Set(UpdateState state)
    {
        State = state;
        Changed?.Invoke();
    }

    public void Dispose() => _http.Dispose();
}
