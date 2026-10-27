using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Storage;
using Windows.Storage.FileProperties;

namespace Poise.App.UI.Services;

/// <summary>从程序文件取图标（系统缩略图），按路径缓存。</summary>
public static class AppIcons
{
    private static readonly Dictionary<string, ImageSource?> Cache = new(StringComparer.OrdinalIgnoreCase);

    public static async Task<ImageSource?> LoadAsync(string path)
    {
        if (Cache.TryGetValue(path, out var cached))
        {
            return cached;
        }
        ImageSource? image = null;
        try
        {
            var file = await StorageFile.GetFileFromPathAsync(path);
            using var thumb = await file.GetThumbnailAsync(ThumbnailMode.SingleItem, 32);
            if (thumb is not null)
            {
                var bitmap = new BitmapImage();
                await bitmap.SetSourceAsync(thumb);
                image = bitmap;
            }
        }
        catch (Exception e) when (e is FileNotFoundException or UnauthorizedAccessException or ArgumentException or System.Runtime.InteropServices.COMException)
        {
            // 取不到图标就不显示，不影响使用。
        }
        Cache[path] = image;
        return image;
    }
}
