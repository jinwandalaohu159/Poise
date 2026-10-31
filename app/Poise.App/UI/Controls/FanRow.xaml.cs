using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Poise.Core.Presentation;

namespace Poise.App.UI.Controls;

/// <summary>
/// 风扇一行：显示转速，滑块调加速（0 = 自动）。
/// 拖动时松手才提交；键盘调节停下 0.4 秒后提交。调节过程中不用服务端的旧值覆盖滑块。
/// </summary>
public sealed partial class FanRow : UserControl
{
    private readonly DispatcherTimer _commit = new() { Interval = TimeSpan.FromMilliseconds(400) };
    private bool _dragging;
    private bool _showing;
    private int _committed = -1;

    public FanRow()
    {
        InitializeComponent();
        Boost.ValueChanged += OnValueChanged;
        Boost.AddHandler(PointerPressedEvent, new PointerEventHandler((_, _) => _dragging = true), true);
        Boost.AddHandler(PointerReleasedEvent, new PointerEventHandler((_, _) => EndDrag()), true);
        Boost.PointerCaptureLost += (_, _) => EndDrag();
        _commit.Tick += (_, _) => Commit();
        // 离开页面时还没提交的调节（键盘调完不到 0.4 秒、拖着没松手）照样提交，不悄悄丢掉。
        Unloaded += (_, _) =>
        {
            _dragging = false;
            if (_commit.IsEnabled || (_committed >= 0 && (int)Math.Round(Boost.Value) != _committed))
            {
                Commit();
            }
        };
    }

    /// <summary>用户选定了加速（0～100，0 = 自动）。</summary>
    public event Action<int>? BoostChosen;

    /// <summary>用户是否正在调节（期间不应用服务端的值）。</summary>
    private bool Adjusting => _dragging || _commit.IsEnabled;

    public void Show(FanView fan)
    {
        SpeedText.Text = fan.Speed;
        if (Adjusting)
        {
            return;
        }
        _showing = true;
        Boost.Value = fan.Boost;
        _committed = fan.Boost;
        ValueText.Text = FanView.BoostText(fan.Boost);
        _showing = false;
    }

    private void OnValueChanged(object sender, RangeBaseValueChangedEventArgs e)
    {
        if (_showing)
        {
            return;
        }
        ValueText.Text = FanView.BoostText((int)Boost.Value);
        if (!_dragging)
        {
            _commit.Stop();
            _commit.Start();
        }
    }

    private void EndDrag()
    {
        if (!_dragging)
        {
            return;
        }
        _dragging = false;
        Commit();
    }

    private void Commit()
    {
        _commit.Stop();
        var value = (int)Math.Round(Boost.Value);
        if (value != _committed)
        {
            _committed = value;
            BoostChosen?.Invoke(value);
        }
    }
}
