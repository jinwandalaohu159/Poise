using CommunityToolkit.Mvvm.ComponentModel;
using Poise.Core.Presentation;

namespace Poise.App.UI.ViewModels;

/// <summary>设置页硬件读数的一行：名称不变，数值随状态刷新原地更新（不重建这一行）。</summary>
public sealed partial class HardwareRowItem(HardwareRow row) : ObservableObject
{
    public string Label { get; } = row.Label;

    [ObservableProperty]
    public partial string Value { get; set; } = row.Value;
}
