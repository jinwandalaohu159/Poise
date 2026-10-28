namespace Poise.Core.Updates;

/// <summary>
/// 已安装的版本：安装目录里的 version.txt（构建脚本写入，与服务的版本号同源）。
/// 开发时单独运行界面没有这个文件，视为开发版本：不检查更新、不联网。
/// </summary>
public static class AppVersion
{
    public static Version? Installed { get; } = Read(Path.Combine(AppContext.BaseDirectory, "version.txt"));

    internal static Version? Read(string path)
    {
        try
        {
            return File.Exists(path) ? Parse(File.ReadAllText(path)) : null;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            return null;
        }
    }

    /// <summary>"0.2.1"、"v0.2.1" 都可以；只取前三段，后缀（如 "-beta"）忽略。</summary>
    public static Version? Parse(string? text)
    {
        var core = text?.Trim().TrimStart('v', 'V').Split('-', '+')[0];
        return Version.TryParse(core, out var v) ? new Version(v.Major, v.Minor, Math.Max(v.Build, 0)) : null;
    }
}
