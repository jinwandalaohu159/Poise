using System.Numerics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Poise.App.UI.Theme;
using Poise.Core.Ipc;

namespace Poise.App.UI.Controls;

/// <summary>
/// 细进度条。<see cref="Fraction"/> 为主段长度（0～1）；<see cref="SecondaryFraction"/> 在主段末端之前
/// 以浅色标出一段（例如已用内存中可以释放的部分）。
/// 第一次显示时直接画到位（不从满格或空格动画过来），之后的变化才平滑过渡。
/// </summary>
public sealed partial class MeterBar : UserControl
{
    private double _fraction;
    private double _secondary;
    private bool _attention;
    /// <summary>已经设过数值（还没有数据时只是空轨道）。</summary>
    private bool _hasValue;
    /// <summary>已经按数值画过一次；之后的变化才有过渡动画。</summary>
    private bool _placed;

    public MeterBar()
    {
        InitializeComponent();
        // 先设好起点再挂过渡：否则会从默认的满格动画缩回来。
        Fill.Scale = new Vector3(0, 1, 1);
        Secondary.Scale = new Vector3(0, 1, 1);
        SizeChanged += (_, _) => Update();
    }

    /// <summary>主段长度，0～1。</summary>
    public double Fraction
    {
        get => _fraction;
        set
        {
            _fraction = Math.Clamp(double.IsFinite(value) ? value : 0, 0, 1);
            _hasValue = true;
            Update();
        }
    }

    /// <summary>主段末端之前以浅色标出的长度，0～1。</summary>
    public double SecondaryFraction
    {
        get => _secondary;
        set
        {
            _secondary = Math.Clamp(double.IsFinite(value) ? value : 0, 0, 1);
            Update();
        }
    }

    /// <summary>按等级着色；null 用强调色。</summary>
    public void SetLevel(Level? level) =>
        VisualStateManager.GoToState(this, level?.ToString() ?? "Accent", true);

    /// <summary>中性灰色（例如排行里的普通项）。</summary>
    public void SetNeutral() => VisualStateManager.GoToState(this, "Neutral", true);

    /// <summary>列表里使用：需要关注时为橙色，否则为中性灰色。可在数据模板里绑定。</summary>
    public bool Attention
    {
        get => _attention;
        set
        {
            _attention = value;
            VisualStateManager.GoToState(this, value ? "Notice" : "Neutral", false);
        }
    }

    private void Update()
    {
        var width = (float)ActualWidth;
        if (width <= 0)
        {
            return;
        }
        // 浅色段画在主段的末端：主段先画到 (fraction - secondary)，浅色段补到 fraction。
        var solid = Math.Max(0, _fraction - _secondary);
        Fill.Scale = new Vector3((float)solid, 1, 1);
        Secondary.Scale = new Vector3((float)_fraction, 1, 1);
        if (_hasValue && !_placed)
        {
            _placed = true;
            Motion.EnableScaleTransition(Fill, Motion.Slow);
            Motion.EnableScaleTransition(Secondary, Motion.Slow);
        }
    }
}
