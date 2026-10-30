using Microsoft.UI.Xaml;
using Poise.Core.Presentation;

namespace Poise.App.UI.Controls;

/// <summary>数据模板里按内容显示或收起元素（x:Bind 函数），不用为每种情况写转换器。</summary>
public static class Show
{
    public static Visibility If(bool visible) => visible ? Visibility.Visible : Visibility.Collapsed;

    public static Visibility IfText(string? text) => If(!string.IsNullOrEmpty(text));

    /// <summary>需要注意的（橙色）。</summary>
    public static Visibility IfNotice(TagTone tone) => If(tone == TagTone.Notice);

    public static Visibility UnlessNotice(TagTone tone) => If(tone != TagTone.Notice);
}
