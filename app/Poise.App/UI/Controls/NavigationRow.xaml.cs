using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;

namespace Poise.App.UI.Controls;

/// <summary>整行可点的设置卡：图标、标题、说明、右侧箭头。点击跳转页面或弹出浮层。</summary>
public sealed partial class NavigationRow : UserControl
{
    public NavigationRow() => InitializeComponent();

    public event RoutedEventHandler? Click;

    public string Glyph
    {
        get => Icon.Glyph;
        set => Icon.Glyph = value;
    }

    public string Title
    {
        get => TitleText.Text;
        set
        {
            TitleText.Text = value;
            AutomationProperties.SetName(Button, value);
        }
    }

    public string Description
    {
        get => DescriptionText.Text;
        set => DescriptionText.Text = value;
    }

    /// <summary>点击时弹出的浮层（代替跳转）。</summary>
    public FlyoutBase? Flyout
    {
        get => Button.Flyout;
        set => Button.Flyout = value;
    }

    private void OnClick(object sender, RoutedEventArgs e) => Click?.Invoke(this, e);
}
