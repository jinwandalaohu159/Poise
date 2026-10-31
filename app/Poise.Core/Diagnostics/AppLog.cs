using System.Globalization;
using System.Text;

namespace Poise.Core.Diagnostics;

/// <summary>
/// 界面的日志：%LOCALAPPDATA%\Poise\app.log（超过 1MB 时清空重写）。记录出错和意外情况，便于排查；
/// 写日志本身出错时静默忽略，绝不影响界面。
/// </summary>
public static class AppLog
{
    private const long MaxBytes = 1024 * 1024;
    private static readonly Lock Gate = new();
    private static string? _path;
    /// <summary>每写这么多行看一次大小。</summary>
    private const int CheckEvery = 100;
    private static int _written;

    public static void Init(string? directory = null)
    {
        var dir = directory ?? Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Poise");
        try
        {
            Directory.CreateDirectory(dir);
            _path = Path.Combine(dir, "app.log");
            if (File.Exists(_path) && new FileInfo(_path).Length > MaxBytes)
            {
                File.Delete(_path);
            }
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            _path = null;
        }
    }

    public static void Info(string message) => Write("INFO", message);

    public static void Warn(string message) => Write("WARN", message);

    public static void Error(string message, Exception? e = null) =>
        Write("ERROR", e is null ? message : $"{message}: {e}");

    private static void Write(string level, string message)
    {
        if (_path is null)
        {
            return;
        }
        var line = string.Create(CultureInfo.InvariantCulture, $"{DateTime.Now:yyyy-MM-dd HH:mm:ss} [{level}] {message}{Environment.NewLine}");
        lock (Gate)
        {
            try
            {
                // 运行期间也有上限（一直开着、某个错误反复出现时）：写满了换成 app.1.log，现场还在。
                if (++_written % CheckEvery == 0 && new FileInfo(_path).Length > MaxBytes)
                {
                    File.Move(_path, Path.ChangeExtension(_path, ".1.log"), overwrite: true);
                }
                File.AppendAllText(_path, line, Encoding.UTF8);
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
            }
        }
    }
}
