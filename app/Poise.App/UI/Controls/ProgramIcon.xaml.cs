using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Poise.App.UI.Services;

namespace Poise.App.UI.Controls;

/// <summary>按程序路径显示图标（系统缩略图，按路径缓存，见 <see cref="AppIcons"/>）；没有路径或取不到时显示通用图标。</summary>
public sealed partial class ProgramIcon : UserControl
{
    public static readonly DependencyProperty PathProperty = DependencyProperty.Register(
        nameof(Path), typeof(string), typeof(ProgramIcon),
        new PropertyMetadata(null, (d, e) => _ = ((ProgramIcon)d).LoadAsync(e.NewValue as string)));

    public ProgramIcon() => InitializeComponent();

    public string? Path
    {
        get => (string?)GetValue(PathProperty);
        set => SetValue(PathProperty, value);
    }

    private async Task LoadAsync(string? path)
    {
        Picture.Source = null;
        Fallback.Visibility = Visibility.Visible;
        if (string.IsNullOrEmpty(path))
        {
            return;
        }
        var image = await AppIcons.LoadAsync(path);
        // 加载期间换了程序（列表行被复用）：丢弃旧的结果。
        if (image is not null && path == Path)
        {
            Picture.Source = image;
            Fallback.Visibility = Visibility.Collapsed;
        }
    }
}
