using Microsoft.UI.Xaml.Markup;
using Poise.Core.Localization;

namespace Poise.App.UI.Localization;

/// <summary>XAML 中取本地化文案：<c>Text="{l:L Key=home.clean}"</c>。</summary>
[MarkupExtensionReturnType(ReturnType = typeof(string))]
public sealed partial class L : MarkupExtension
{
    public string Key { get; set; } = "";

    protected override object ProvideValue() => Loc.Get(Key);
}
