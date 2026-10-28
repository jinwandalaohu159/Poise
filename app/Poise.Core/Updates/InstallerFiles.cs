using System.Security.Cryptography;

namespace Poise.Core.Updates;

/// <summary>
/// 下载的安装程序放在 %LOCALAPPDATA%\Poise\updates：按版本命名，校验大小和 SHA-256，旧版本的安装程序及时删除。
/// </summary>
public sealed class InstallerFiles(string directory)
{
    public static InstallerFiles Default { get; } = new(Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Poise", "updates"));

    public string Directory { get; } = directory;

    public string PathFor(Version version) => Path.Combine(Directory, $"Poise-Setup-{version.ToString(3)}.exe");

    /// <summary>下载过程中的临时文件，校验通过后才改名为 <see cref="PathFor"/>。</summary>
    public string PartialPathFor(Version version) => PathFor(version) + ".part";

    /// <summary>文件存在，大小和 SHA-256 都一致。</summary>
    public static bool Matches(string path, long size, string sha256)
    {
        using var verified = OpenVerified(path, size, sha256);
        return verified is not null;
    }

    /// <summary>
    /// 打开并校验文件（大小与 SHA-256）。校验通过时返回保持打开的文件：在它关闭之前，别的程序不能改写或替换
    /// 这个文件，从校验到启动安装程序之间不会被调包。不一致或打不开时返回 null。
    /// </summary>
    public static FileStream? OpenVerified(string path, long size, string sha256)
    {
        FileStream? stream = null;
        try
        {
            stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read);
            if (stream.Length != size
                || !string.Equals(Convert.ToHexStringLower(SHA256.HashData(stream)), sha256, StringComparison.OrdinalIgnoreCase))
            {
                stream.Dispose();
                return null;
            }
            return stream;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            stream?.Dispose();
            return null;
        }
    }

    public static string Sha256Of(string path)
    {
        using var stream = File.OpenRead(path);
        return Convert.ToHexStringLower(SHA256.HashData(stream));
    }

    /// <summary>删除除 <paramref name="keep"/> 以外的安装程序和未下载完的文件。</summary>
    public void DeleteAllExcept(string? keep)
    {
        try
        {
            if (!System.IO.Directory.Exists(Directory))
            {
                return;
            }
            foreach (var file in System.IO.Directory.EnumerateFiles(Directory, "Poise-Setup-*"))
            {
                if (!string.Equals(file, keep, StringComparison.OrdinalIgnoreCase))
                {
                    TryDelete(file);
                }
            }
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            // 清理旧文件失败不影响更新，下次再删。
        }
    }

    public static void TryDelete(string path)
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
