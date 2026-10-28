using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Poise.Core.Presentation;

namespace Poise.App.UI.Controls;

/// <summary>四个核心指标一行。紧凑模式（托盘面板）间距更小、不显示进度条、数字小一号。</summary>
public sealed partial class MetricStrip : UserControl
{
    public MetricStrip()
    {
        InitializeComponent();
    }

    public bool IsCompact
    {
        set
        {
            Root.ColumnSpacing = value ? 12 : 32;
            Root.Padding = value ? new Thickness(2, 0, 2, 0) : default;
            foreach (var tile in Tiles)
            {
                tile.IsCompact = value;
            }
        }
    }

    private MetricTile[] Tiles => [CpuLoadTile, CpuTempTile, GpuTempTile, MemoryTile];

    public void Show(HomeView v)
    {
        CpuLoadTile.Show(v.CpuLoad);
        CpuTempTile.Show(v.CpuTemp);
        GpuTempTile.Show(v.GpuTemp);
        MemoryTile.Show(v.Memory);
    }
}
