using System.Net;
using System.Text;
using Poise.Core.Presentation;
using Poise.Core.Updates;

namespace Poise.Core.Tests;

public sealed class UpdateTests
{
    private const string Digest = "002ab13d0f78e78635d8b2ce5357affe4cb72f1bd00ec003333bb369fde8434b";

    private static string Release(string tag = "v0.3.0", bool prerelease = false, bool draft = false, string assets = "") =>
        $$"""
        {
          "tag_name": "{{tag}}",
          "draft": {{(draft ? "true" : "false")}},
          "prerelease": {{(prerelease ? "true" : "false")}},
          "html_url": "https://github.com/jinwandalaohu159/Poise/releases/tag/{{tag}}",
          "body": "## 安装\n\n- **还原到系统默认**：说明\n- 开机启动更快\n- 第三条\n- 第四条",
          "assets": [{{assets}}]
        }
        """;

    private static string Asset(string name, long size = 100, string? digest = "sha256:" + Digest) =>
        $$"""
        { "name": "{{name}}", "size": {{size}},
          "browser_download_url": "https://github.com/jinwandalaohu159/Poise/releases/download/v0.3.0/{{name}}"
          {{(digest is null ? "" : $", \"digest\": \"{digest}\"")}} }
        """;

    [Fact]
    public void Latest_release_picks_the_installer_and_its_digest()
    {
        var json = Release(assets: $"{Asset("notes.txt")}, {Asset("Poise-Setup-0.2.9.exe", 1)}, {Asset("Poise-Setup-0.3.0.exe", 106446848)}");

        var r = GitHubReleases.ParseLatest(json)!;

        Assert.Equal(new Version(0, 3, 0), r.Version);
        Assert.Equal("v0.3.0", r.Tag);
        Assert.EndsWith("/Poise-Setup-0.3.0.exe", r.InstallerUrl.AbsoluteUri);
        Assert.Equal(106446848, r.InstallerSize);
        Assert.Equal(Digest, r.Sha256);
        Assert.Equal("https://github.com/jinwandalaohu159/Poise/releases/tag/v0.3.0", r.PageUrl.AbsoluteUri);
    }

    [Fact]
    public void Releases_without_an_installer_or_not_final_are_ignored()
    {
        Assert.Null(GitHubReleases.ParseLatest(Release(assets: Asset("notes.txt"))));
        Assert.Null(GitHubReleases.ParseLatest(Release(prerelease: true, assets: Asset("Poise-Setup-0.3.0.exe"))));
        Assert.Null(GitHubReleases.ParseLatest(Release(draft: true, assets: Asset("Poise-Setup-0.3.0.exe"))));
        Assert.Null(GitHubReleases.ParseLatest(Release(tag: "nightly", assets: Asset("Poise-Setup-0.3.0.exe"))));
        Assert.Null(GitHubReleases.ParseLatest("not json"));
        // 安装程序的名称和版本对不上（例如只有旧版本的安装程序）：不认。
        Assert.Null(GitHubReleases.ParseLatest(Release(assets: Asset("Poise-Setup-0.2.9.exe"))));
    }

    [Fact]
    public void Releases_without_a_sha256_digest_are_not_offered()
    {
        Assert.Null(GitHubReleases.ParseLatest(Release(assets: Asset("Poise-Setup-0.3.0.exe", digest: null))));
        Assert.Null(GitHubReleases.Sha256("md5:abc"));
        Assert.Null(GitHubReleases.Sha256("sha256:xyz"));
        Assert.Equal(Digest, GitHubReleases.Sha256("SHA256:" + Digest.ToUpperInvariant()));
    }

    [Theory]
    [InlineData("0.2.1", "0.2.0", true)]
    [InlineData("v0.2.10", "0.2.9", true)]
    [InlineData("0.3.0", "0.3.0", false)]
    [InlineData("1.0.0-beta", "0.9.9", true)]
    [InlineData("0.2.0", "0.2.1", false)]
    public void Versions_compare_numerically(string latest, string installed, bool newer) =>
        Assert.Equal(newer, AppVersion.Parse(latest) > AppVersion.Parse(installed));

    [Fact]
    public void Version_file_is_trimmed_and_missing_file_means_development_build()
    {
        var dir = Directory.CreateTempSubdirectory("poise-version-");
        try
        {
            var file = Path.Combine(dir.FullName, "version.txt");
            Assert.Null(AppVersion.Read(file));
            File.WriteAllText(file, "0.2.1\r\n");
            Assert.Equal(new Version(0, 2, 1), AppVersion.Read(file));
        }
        finally
        {
            dir.Delete(recursive: true);
        }
    }

    [Fact]
    public void The_updater_is_copied_out_of_the_install_directory()
    {
        var root = Directory.CreateTempSubdirectory("poise-update-files-");
        try
        {
            var install = Directory.CreateDirectory(Path.Combine(root.FullName, "Poise"));
            var files = new UpdateFiles(Path.Combine(root.FullName, "updates"), install.FullName + Path.DirectorySeparatorChar);
            Assert.Equal(install.FullName, files.InstallDirectory);
            Assert.Throws<FileNotFoundException>(files.PrepareUpdater);

            File.WriteAllText(Path.Combine(install.FullName, "poise-updater.exe"), "v2");
            var copy = files.PrepareUpdater();
            Assert.Equal(Path.Combine(files.Directory, "poise-updater.exe"), copy);
            Assert.Equal("v2", File.ReadAllText(copy));

            // 已是最新版本时删除下载过的安装程序，更新助手本身保留。
            File.WriteAllText(Path.Combine(files.Directory, "Poise-Setup-0.3.0.exe"), "setup");
            File.WriteAllText(Path.Combine(files.Directory, "Poise-Setup-0.3.1.exe.part"), "partial");
            files.DeleteInstallers();
            Assert.Equal([copy], Directory.GetFiles(files.Directory));
        }
        finally
        {
            root.Delete(recursive: true);
        }
    }

    [Fact]
    public async Task Starting_an_update_without_the_updater_fails_and_can_be_retried()
    {
        var json = Release(assets: Asset("Poise-Setup-0.3.0.exe"));
        using var updater = new Updater(new Version(0, 2, 1), Scratch(), new Reply(HttpStatusCode.OK, json));
        var started = false;
        updater.UpdaterStarted += () => started = true;
        await updater.CheckAsync();

        updater.StartUpdate();

        Assert.False(started);
        Assert.Equal(UpdatePhase.Failed, updater.State.Phase);
        Assert.Equal(UpdateError.Launch, updater.State.Error);
        Assert.True(updater.State.CanInstall);
    }

    [Fact]
    public async Task Check_reports_a_newer_release_and_up_to_date()
    {
        var json = Release(assets: Asset("Poise-Setup-0.3.0.exe"));
        using var older = new Updater(new Version(0, 2, 1), Scratch(), new Reply(HttpStatusCode.OK, json));
        await older.CheckAsync();
        Assert.Equal(UpdatePhase.Available, older.State.Phase);
        Assert.True(older.State.CanInstall);

        using var same = new Updater(new Version(0, 3, 0), Scratch(), new Reply(HttpStatusCode.OK, json));
        await same.CheckAsync();
        Assert.Equal(UpdatePhase.UpToDate, same.State.Phase);
        Assert.False(same.State.CanInstall);
    }

    [Fact]
    public async Task Failed_checks_are_shown_only_when_asked_for()
    {
        using var quiet = new Updater(new Version(0, 2, 1), Scratch(), new Reply(HttpStatusCode.InternalServerError, ""));
        await quiet.CheckAsync(quiet: true);
        Assert.Equal(UpdatePhase.Idle, quiet.State.Phase);

        await quiet.CheckAsync();
        Assert.Equal(UpdatePhase.Failed, quiet.State.Phase);
        Assert.Equal(UpdateError.Network, quiet.State.Error);
    }

    [Fact]
    public async Task Development_builds_never_go_online()
    {
        var reply = new Reply(HttpStatusCode.OK, Release(assets: Asset("Poise-Setup-0.3.0.exe")));
        using var dev = new Updater(null, Scratch(), reply);
        await dev.CheckAsync();
        Assert.Equal(UpdatePhase.Unavailable, dev.State.Phase);
        Assert.Equal(0, reply.Calls);
        Assert.False(dev.State.CanCheck);
    }

    [Fact]
    public void Status_text_and_notes_summary()
    {
        TestSupport.UseChinese();
        var release = GitHubReleases.ParseLatest(Release(assets: Asset("Poise-Setup-0.3.0.exe")))!;

        Assert.Equal("发现新版本 0.3.0", UpdateText.Status(new UpdateState(UpdatePhase.Available, release)));
        Assert.Equal("正在启动更新…", UpdateText.Status(new UpdateState(UpdatePhase.Starting, release)));
        Assert.Equal("安装\n还原到系统默认：说明\n开机启动更快", UpdateText.Notes(new UpdateState(UpdatePhase.Available, release)));
        Assert.Equal("", UpdateText.Notes(new UpdateState(UpdatePhase.UpToDate)));
        Assert.Equal("Poise 0.2.1", UpdateText.VersionLine(new Version(0, 2, 1)));
    }

    /// <summary>不存在的临时目录：没有更新助手。</summary>
    private static UpdateFiles Scratch()
    {
        var root = Path.Combine(Path.GetTempPath(), "poise-updates-test-" + Guid.NewGuid().ToString("N"));
        return new UpdateFiles(Path.Combine(root, "updates"), Path.Combine(root, "Poise"));
    }

    /// <summary>固定返回一个响应，并记录请求次数。</summary>
    private sealed class Reply(HttpStatusCode status, string body) : HttpMessageHandler
    {
        public int Calls { get; private set; }

        protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancel)
        {
            Calls++;
            Assert.Contains(request.Headers.UserAgent, p => p.Product?.Name == "Poise");
            return Task.FromResult(new HttpResponseMessage(status)
            {
                Content = new StringContent(body, Encoding.UTF8, "application/json"),
            });
        }
    }
}
