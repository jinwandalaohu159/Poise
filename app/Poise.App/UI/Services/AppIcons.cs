using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Storage;
using Windows.Storage.FileProperties;

namespace Poise.App.UI.Services;

/// <summary>
/// 从程序文件取图标（系统缩略图），按路径缓存。正在取的也记着：几个页面、几次刷新同时要同一个图标时
/// 只取一次。只在界面线程上使用。
/// </summary>
public static class AppIcons
{
    /// <summary>缓存多于这么多个图标时，丢掉已经读完的（需要时再读）。</summary>
    private const int MaxCached = 512;

    private static readonly Dictionary<string, Task<ImageSource?>> Cache = new(StringComparer.OrdinalIgnoreCase);

    public static Task<ImageSource?> LoadAsync(string path)
    {
        if (!Cache.TryGetValue(path, out var icon))
        {
            if (Cache.Count >= MaxCached)
            {
                foreach (var done in Cache.Where(p => p.Value.IsCompleted).Select(p => p.Key).ToList())
                {
                    Cache.Remove(done);
                }
            }
            icon = ReadAsync(path);
            Cache[path] = icon;
        }
        return icon;
    }

    /// <summary>已经取到的图标（不用等）；还没取或取不到时为空。</summary>
    public static ImageSource? Cached(string path) =>
        Cache.TryGetValue(path, out var icon) && icon.IsCompletedSuccessfully ? icon.Result : null;

    private static async Task<ImageSource?> ReadAsync(string path)
    {
        try
        {
            var file = await StorageFile.GetFileFromPathAsync(path);
            using var thumb = await file.GetThumbnailAsync(ThumbnailMode.SingleItem, 32);
            if (thumb is null)
            {
                return null;
            }
            var bitmap = new BitmapImage();
            await bitmap.SetSourceAsync(thumb);
            return bitmap;
        }
        catch (Exception e) when (e is FileNotFoundException or UnauthorizedAccessException or ArgumentException or System.Runtime.InteropServices.COMException)
        {
            // 取不到图标就不显示，不影响使用。
            return null;
        }
    }
}
