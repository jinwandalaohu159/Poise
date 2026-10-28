using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Poise.Core.Presentation;

namespace Poise.App.UI.Controls;

/// <summary>状态小标签（清理页的"占用过高"等）。可在数据模板里绑定 <see cref="Text"/> 与 <see cref="Tone"/>。</summary>
public sealed partial class StatusTag : UserControl
{
    public static readonly DependencyProperty TextProperty = DependencyProperty.Register(
        nameof(Text), typeof(string), typeof(StatusTag),
        new PropertyMetadata("", (d, e) => ((StatusTag)d).Label.Text = (string)e.NewValue));

    public static readonly DependencyProperty ToneProperty = DependencyProperty.Register(
        nameof(Tone), typeof(TagTone), typeof(StatusTag),
        new PropertyMetadata(TagTone.None, (d, e) => ((StatusTag)d).ApplyTone()));

    public StatusTag()
    {
        InitializeComponent();
        Loaded += (_, _) => ApplyTone();
    }

    public string Text
    {
        get => (string)GetValue(TextProperty);
        set => SetValue(TextProperty, value);
    }

    public TagTone Tone
    {
        get => (TagTone)GetValue(ToneProperty);
        set => SetValue(ToneProperty, value);
    }

    private void ApplyTone() => VisualStateManager.GoToState(this, Tone.ToString(), false);
}
