using System.Numerics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using Poise.App.UI.Services;
using Poise.App.UI.Theme;
using Poise.Core.Monitoring;
using Windows.Foundation;

namespace Poise.App.UI.Controls;

/// <summary>
/// 最近几分钟的走势：性能核（蓝，带浅色填充）、能效核（绿）、内存（紫）、CPU 温度（橙，°C），共用 0～100 刻度；
/// 不是大小核处理器时，蓝线为整机 CPU，没有绿线。性能核负载达到 <see cref="HighLoadPct"/> 的时段用浅蓝底标出。
/// <para>
/// 位置按本地时钟计算（见 <see cref="TrendSeries.X"/>）：每次换上新数据时按"此刻"重画，
/// 两次重画之间曲线层按时钟逐像素左移。数据到得早或晚都不会让曲线停顿或跳动，左右两端也始终有数据。
/// </para>
/// <para>
/// 曲线每秒只移动几像素，不用逐帧动画（每秒 60 次重绘，开销与效果不成比例）：
/// 每走过一个物理像素的时间才更新一次位置，界面只在这时重绘。
/// </para>
/// </summary>
public sealed partial class LineChart : UserControl
{
    private const double HighLoadPct = 80;
    /// <summary>超过这么久没有新数据（窗口隐藏、服务停止）就停止平移，有新数据时再继续。</summary>
    private static readonly TimeSpan Stale = TimeSpan.FromSeconds(10);

    private readonly DispatcherQueueTimer _scroll;
    private TrendSeries _series = TrendSeries.Empty;
    /// <summary>当前曲线是按哪个时刻画的。</summary>
    private TimeSpan _drawnAt;
    /// <summary>这次重画用到了几个高负载底色矩形（其余的删掉）。</summary>
    private int _bandsUsed;

    public LineChart()
    {
        InitializeComponent();
        Plot.OpacityTransition = new ScalarTransition { Duration = Motion.Enabled ? Motion.Fast : Motion.Instant };
        _scroll = DispatcherQueue.GetForCurrentThread().CreateTimer();
        _scroll.Tick += (_, _) => Scroll();
        ActualThemeChanged += (_, _) => DrawGrid(PlotArea.ActualWidth, PlotArea.ActualHeight);
        Unloaded += (_, _) => _scroll.Stop();
    }

    public string StartText
    {
        set => StartLabel.Text = value;
    }

    public string EndText
    {
        set => EndLabel.Text = value;
    }

    /// <summary>换上新数据。从没有数据到有数据时淡入，之后只是接着画。</summary>
    public void Show(TrendSeries series)
    {
        var appearing = _series.Samples.Count == 0 && series.Samples.Count > 0;
        _series = series;
        Redraw();
        if (appearing)
        {
            Plot.Opacity = 0;
            Plot.Opacity = 1;
        }
    }

    private void OnPlotSizeChanged(object sender, SizeChangedEventArgs e)
    {
        // 线宽会伸出绘图区上下边缘一点，裁剪区上下各留几像素。
        PlotArea.Clip = new RectangleGeometry { Rect = new Rect(0, -4, e.NewSize.Width, e.NewSize.Height + 8) };
        DrawGrid(e.NewSize.Width, e.NewSize.Height);
        Redraw();
    }

    private void DrawGrid(double w, double h)
    {
        GridLayer.Children.Clear();
        var brush = GridBrush.Background;
        foreach (var pct in new[] { 0.0, 25, 50, 75, 100 })
        {
            GridLayer.Children.Add(new Line { X1 = 0, X2 = w, Y1 = Y(pct, h), Y2 = Y(pct, h), Stroke = brush, StrokeThickness = 1 });
        }
    }

    /// <summary>按此刻的位置重画曲线，并从这里开始匀速左移。</summary>
    private void Redraw()
    {
        var w = PlotArea.ActualWidth;
        var h = PlotArea.ActualHeight;
        if (w <= 0 || h <= 0)
        {
            return;
        }
        var now = TrendFeed.Now;
        var samples = _series.Samples;
        double X(TrendSample s) => _series.X(s.At, now, w);

        var load = samples.Select(s => new Point(X(s), Y(s.Load, h))).ToList();
        LoadLine.Points = Line(load);
        EfficiencyLine.Points = Line(samples.Where(s => s.Efficiency is not null).Select(s => new Point(X(s), Y(s.Efficiency!.Value, h))));
        MemoryLine.Points = Line(samples.Select(s => new Point(X(s), Y(s.Memory, h))));
        TempLine.Points = Line(samples.Where(s => s.CpuTemp is not null).Select(s => new Point(X(s), Y(s.CpuTemp!.Value, h))));
        LoadArea.Points = Area(load, h);
        DrawBands(samples.Select(s => (X(s), s.Load >= HighLoadPct)).ToList(), h);
        _drawnAt = now;
        Slider.Translation = Vector3.Zero;
        StartScrolling(w);
    }

    /// <summary>每走过一个物理像素更新一次位置（约每秒 4～6 次，随宽度变化）。关闭了动画效果时不平移，每次刷新跳到新位置。</summary>
    private void StartScrolling(double w)
    {
        var pixels = w * (XamlRoot?.RasterizationScale ?? 1);
        if (!Motion.Enabled || pixels <= 0 || _series.Samples.Count == 0)
        {
            _scroll.Stop();
            return;
        }
        _scroll.Interval = TimeSpan.FromSeconds(Math.Clamp(_series.Window.TotalSeconds / pixels, 0.05, 1));
        if (!_scroll.IsRunning)
        {
            _scroll.Start();
        }
    }

    /// <summary>按时钟算出曲线层此刻应左移的距离，对齐到物理像素（线条不会因半像素位置而发虚）。</summary>
    private void Scroll()
    {
        var elapsed = TrendFeed.Now - _drawnAt;
        // 窗口被挡住、最小化时不再平移（重新显示时会拿到新数据、重新开始）。
        if (elapsed > Stale || !AppHost.Current.Presence.MainVisible)
        {
            _scroll.Stop();
            return;
        }
        var scale = XamlRoot?.RasterizationScale ?? 1;
        var dx = PlotArea.ActualWidth * elapsed.TotalSeconds / _series.Window.TotalSeconds;
        Slider.Translation = new Vector3((float)(-Math.Round(dx * scale) / scale), 0, 0);
    }

    private static double Y(double pct, double h) => h * (1 - Math.Clamp(pct, 0, 100) / 100);

    private static PointCollection Line(IEnumerable<Point> points)
    {
        var line = new PointCollection();
        foreach (var p in points)
        {
            line.Add(p);
        }
        return line;
    }

    private static PointCollection Area(List<Point> line, double h)
    {
        var area = new PointCollection();
        if (line.Count < 2)
        {
            return area;
        }
        area.Add(new Point(line[0].X, h));
        foreach (var p in line)
        {
            area.Add(p);
        }
        area.Add(new Point(line[^1].X, h));
        return area;
    }

    /// <summary>连续的高负载点合并成一段底色，延伸到下一个点。已有的矩形原地挪动，多出来的才删掉。</summary>
    private void DrawBands(List<(double X, bool High)> points, double h)
    {
        _bandsUsed = 0;
        double? start = null;
        for (var i = 0; i < points.Count; i++)
        {
            var (x, high) = points[i];
            if (high)
            {
                start ??= x;
            }
            else if (start is { } s)
            {
                AddBand(s, x, h);
                start = null;
            }
        }
        if (start is { } open && points.Count > 1)
        {
            var step = points[^1].X - points[^2].X;
            AddBand(open, points[^1].X + step, h);
        }
        while (Bands.Children.Count > _bandsUsed)
        {
            Bands.Children.RemoveAt(Bands.Children.Count - 1);
        }
    }

    private void AddBand(double from, double to, double h)
    {
        Rectangle band;
        if (_bandsUsed < Bands.Children.Count)
        {
            band = (Rectangle)Bands.Children[_bandsUsed];
        }
        else
        {
            band = new Rectangle { Opacity = 0.08 };
            Bands.Children.Add(band);
        }
        _bandsUsed++;
        band.Width = Math.Max(2, to - from);
        band.Height = h;
        band.Fill = LoadLine.Stroke;
        Canvas.SetLeft(band, from);
    }
}
