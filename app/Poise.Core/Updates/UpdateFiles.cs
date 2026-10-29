namespace Poise.Core.Updates;

/// <summary>
/// 更新用到的文件。
/// <list type="bullet">
/// <item>更新助手 poise-updater.exe 与界面在同一个安装目录里；更新时先复制到 %LOCALAPPDATA%\Poise\updates 再运行，
/// 安装程序替换安装目录里的文件时，它不占用任何文件。</item>
/// <item>更新助手把下载的安装程序也放在这里（Poise-Setup-x.y.z.exe）；已是最新版本时删除。</item>
/// </list>
/// </summary>
public sealed class UpdateFiles(string directory, string installDirectory)
{
    private const string UpdaterName = "poise-updater.exe";

    public static UpdateFiles Default { get; } = new(
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Poise", "updates"),
        AppContext.BaseDirectory);

    public string Directory { get; } = directory;

    /// <summary>已安装的 Poise 所在目录（不带末尾的分隔符）。</summary>
    public string InstallDirectory { get; } = Path.TrimEndingDirectorySeparator(installDirectory);

    /// <summary>
    /// 把更新助手复制到更新目录，返回副本的路径。安装目录里没有更新助手时（例如开发时单独运行界面）
    /// 抛出 <see cref="FileNotFoundException"/>。
    /// </summary>
    public string PrepareUpdater()
    {
        var source = Path.Combine(InstallDirectory, UpdaterName);
        if (!File.Exists(source))
        {
            throw new FileNotFoundException("没有找到更新助手", source);
        }
        System.IO.Directory.CreateDirectory(Directory);
        var copy = Path.Combine(Directory, UpdaterName);
        File.Copy(source, copy, overwrite: true);
        return copy;
    }

    /// <summary>删除下载过的安装程序（已是最新版本，不再需要）。</summary>
    public void DeleteInstallers()
    {
        try
        {
            if (!System.IO.Directory.Exists(Directory))
            {
                return;
            }
            foreach (var file in System.IO.Directory.EnumerateFiles(Directory, "Poise-Setup-*"))
            {
                TryDelete(file);
            }
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            // 清理旧文件失败不影响使用，下次再删。
        }
    }

    private static void TryDelete(string path)
    {
        try
        {
            File.Delete(path);
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            // 正在被使用（例如安装程序还在运行）：下次再删。
        }
    }
}
