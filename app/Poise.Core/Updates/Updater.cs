using System.ComponentModel;
using System.Diagnostics;
using Poise.Core.Diagnostics;

namespace Poise.Core.Updates;

/// <summary>
/// 检查、下载并启动新版本的安装程序。
/// <list type="bullet">
/// <item>只访问 GitHub（查询最新版本、下载安装程序）；开发版本不联网。</item>
/// <item>同一时间只做一件事；方法从不抛异常，结果反映在 <see cref="State"/> 上并记日志。</item>
/// <item>在界面线程上调用，<see cref="Changed"/> 也在界面线程上触发（下载在后台线程进行，进度转回调用线程）。</item>
/// </list>
/// 安装程序自己请求管理员权限，并替换正在运行的 Poise；它启动后触发 <see cref="InstallerStarted"/>，界面应随即退出。
/// </summary>
public sealed class Updater : IDisposable
{
    private static readonly TimeSpan CheckTimeout = TimeSpan.FromSeconds(15);
    private static readonly TimeSpan DownloadTimeout = TimeSpan.FromMinutes(30);
    private const int BufferSize = 81920;

    private readonly Version? _installed;
    private readonly InstallerFiles _files;
    private readonly HttpClient _http;
    private bool _working;

    public Updater(Version? installed, InstallerFiles files, HttpMessageHandler? handler = null)
    {
        _installed = installed;
        _files = files;
        _http = handler is null ? new HttpClient() : new HttpClient(handler);
        // 超时按操作分别设置（查询 15 秒，下载 30 分钟）。
        _http.Timeout = Timeout.InfiniteTimeSpan;
        State = installed is null ? new UpdateState(UpdatePhase.Unavailable) : new UpdateState(UpdatePhase.Idle);
    }

    public UpdateState State { get; private set; }

    public event Action? Changed;

    public event Action? InstallerStarted;

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
                _files.DeleteAllExcept(null);
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

    /// <summary>下载已知的新版本（已经下载过且校验通过时直接用），然后启动安装程序。</summary>
    public async Task DownloadAndInstallAsync()
    {
        if (State.Release is not { } release || _working)
        {
            return;
        }
        _working = true;
        try
        {
            var path = _files.PathFor(release.Version);
            var verified = InstallerFiles.OpenVerified(path, release.InstallerSize, release.Sha256);
            if (verified is null)
            {
                Set(new UpdateState(UpdatePhase.Downloading, release));
                var progress = new Progress<double>(p =>
                {
                    if (State.Phase == UpdatePhase.Downloading)
                    {
                        Set(State with { Progress = p });
                    }
                });
                await Task.Run(() => DownloadAsync(release, path, progress));
                verified = InstallerFiles.OpenVerified(path, release.InstallerSize, release.Sha256)
                           ?? throw new InvalidDataException("安装程序校验不通过");
            }
            // 从校验到启动一直占着文件：中间不会被替换。
            using (verified)
            {
                _files.DeleteAllExcept(path);
                Set(new UpdateState(UpdatePhase.Installing, release));
                Process.Start(new ProcessStartInfo(path) { UseShellExecute = true })?.Dispose();
            }
            AppLog.Info($"已启动 {release.Tag} 的安装程序");
            InstallerStarted?.Invoke();
        }
        catch (InvalidDataException e)
        {
            Fail(release, UpdateError.Verification, e);
        }
        catch (Exception e) when (e is HttpRequestException or OperationCanceledException or IOException or UnauthorizedAccessException)
        {
            Fail(release, UpdateError.Network, e);
        }
        catch (Exception e) when (e is Win32Exception or InvalidOperationException)
        {
            // 包括用户在权限提示里选了"否"。
            Fail(release, UpdateError.Launch, e);
        }
        finally
        {
            _working = false;
        }
    }

    /// <summary>下载到临时文件，校验通过后改名。在后台线程上运行。</summary>
    private async Task DownloadAsync(ReleaseInfo release, string path, IProgress<double> progress)
    {
        Directory.CreateDirectory(_files.Directory);
        var partial = _files.PartialPathFor(release.Version);
        using var cancel = new CancellationTokenSource(DownloadTimeout);
        using (var response = await _http.GetAsync(release.InstallerUrl, HttpCompletionOption.ResponseHeadersRead, cancel.Token))
        {
            response.EnsureSuccessStatusCode();
            await using var source = await response.Content.ReadAsStreamAsync(cancel.Token);
            await using var target = File.Create(partial);
            var buffer = new byte[BufferSize];
            long done = 0;
            var reported = -1;
            int read;
            while ((read = await source.ReadAsync(buffer, cancel.Token)) > 0)
            {
                done += read;
                if (done > release.InstallerSize)
                {
                    throw new InvalidDataException("下载的文件比发布信息里的大");
                }
                await target.WriteAsync(buffer.AsMemory(0, read), cancel.Token);
                var percent = (int)(done * 100 / release.InstallerSize);
                if (percent != reported)
                {
                    reported = percent;
                    progress.Report(done / (double)release.InstallerSize);
                }
            }
        }
        if (!InstallerFiles.Matches(partial, release.InstallerSize, release.Sha256))
        {
            InstallerFiles.TryDelete(partial);
            throw new InvalidDataException("下载的安装程序大小或校验值不符");
        }
        File.Move(partial, path, overwrite: true);
    }

    private void Fail(ReleaseInfo release, UpdateError error, Exception e)
    {
        AppLog.Warn($"更新到 {release.Tag} 失败（{error}）：{e.Message}");
        Set(new UpdateState(UpdatePhase.Failed, release, Error: error));
    }

    private void Set(UpdateState state)
    {
        State = state;
        Changed?.Invoke();
    }

    public void Dispose() => _http.Dispose();
}
