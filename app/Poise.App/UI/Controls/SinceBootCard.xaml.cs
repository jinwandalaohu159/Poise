using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Poise.Core.Presentation;

namespace Poise.App.UI.Controls;

/// <summary>清理页"与开机时相比"：整机句柄、线程、内存等总量与开机基线逐项对比。</summary>
public sealed partial class SinceBootCard : UserControl
{
    private SinceBootView? _shown;

    public SinceBootCard() => InitializeComponent();

    /// <summary>增长明显的一项用提醒色标出变化量。</summary>
    public static TagTone ToneOf(bool notable) => notable ? TagTone.Notice : TagTone.None;

    public void Show(SinceBootView v)
    {
        // 每次刷新都会拿到新的对象；内容没变时不重建列表。
        if (_shown is { } old && old.Caption == v.Caption && old.Rows.SequenceEqual(v.Rows))
        {
            return;
        }
        _shown = v;
        CaptionText.Text = v.Caption;
        var hasRows = v.Rows.Count > 0;
        Rows.ItemsSource = v.Rows;
        Table.Visibility = hasRows ? Visibility.Visible : Visibility.Collapsed;
    }
}
