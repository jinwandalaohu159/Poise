using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Imaging;

namespace Poise.App.UI.Controls;

/// <summary>Poise 标志。尺寸由使用处的 Width / Height 决定（标题栏 16，设置页 32）。</summary>
public sealed partial class LogoMark : UserControl
{
    /// <summary>所有标志共用一张位图。非打包应用直接从程序目录读取。</summary>
    private static readonly Uri Source = new(Path.Combine(AppContext.BaseDirectory, "Assets", "Poise.png"));

    public LogoMark()
    {
        InitializeComponent();
        Mark.Source = new BitmapImage(Source);
    }
}
