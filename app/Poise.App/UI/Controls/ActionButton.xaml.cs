using System.Numerics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Poise.App.UI.Theme;

namespace Poise.App.UI.Controls;

/// <summary>
/// 一键操作按钮：点击后文字换成进度环和"正在…"，完成后变成对勾，再回到原文字。不弹任何窗口。
/// </summary>
public sealed partial class ActionButton : UserControl
{
    public ActionButton()
    {
        InitializeComponent();
        foreach (var e in new UIElement[] { IdleContent, BusyContent, DoneIcon })
        {
            Motion.EnableTransitions(e, Motion.Fast);
        }
        Motion.AddPressFeedback(Button);
    }

    public string Label
    {
        get => LabelText.Text;
        set
        {
            LabelText.Text = value;
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(Button, value);
        }
    }

    public string BusyLabel
    {
        get => BusyText.Text;
        set => BusyText.Text = value;
    }

    public string Glyph
    {
        get => Icon.Glyph;
        set => Icon.Glyph = value;
    }

    public bool IsAccent
    {
        set => Button.Style = (Style)Application.Current.Resources[value ? "AccentActionButtonStyle" : "ActionButtonStyle"];
    }

    public bool IsBusy { get; private set; }

    /// <summary>点击时执行的操作。</summary>
    public Func<Task>? Action { get; set; }

    private async void OnClick(object sender, RoutedEventArgs e)
    {
        if (IsBusy || Action is null)
        {
            return;
        }
        IsBusy = BusyRing.IsActive = true;
        Show(BusyContent);
        try
        {
            await Action();
            Show(DoneIcon);
            DoneIcon.Scale = new Vector3(0.6f, 0.6f, 1);
            DoneIcon.CenterPoint = new Vector3((float)DoneIcon.ActualWidth / 2, (float)DoneIcon.ActualHeight / 2, 0);
            DoneIcon.Scale = Vector3.One;
            await Task.Delay(900);
        }
        finally
        {
            Show(IdleContent);
            IsBusy = BusyRing.IsActive = false;
        }
    }

    private void Show(UIElement visible)
    {
        foreach (var e in new UIElement[] { IdleContent, BusyContent, DoneIcon })
        {
            e.Opacity = e == visible ? 1 : 0;
        }
    }
}
