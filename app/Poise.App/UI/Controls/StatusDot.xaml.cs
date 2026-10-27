using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Poise.Core.Ipc;

namespace Poise.App.UI.Controls;

/// <summary>8px 状态圆点：绿 / 橙 / 红 / 灰（未连接）。</summary>
public sealed partial class StatusDot : UserControl
{
    public StatusDot() => InitializeComponent();

    /// <summary>null 表示未连接。</summary>
    public void Show(Level? level) =>
        VisualStateManager.GoToState(this, level?.ToString() ?? "Off", true);
}
