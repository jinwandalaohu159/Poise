using Poise.Core.Ipc;
using Poise.Core.Monitoring;

namespace Poise.Core.Tests;

/// <summary>走势的时刻换算：曲线位置只取决于本地时钟，不随请求快慢跳动。</summary>
public sealed class TrendSeriesTests
{
    private static HistoryPoint Point(float secsAgo, float cpu = 10) => new(secsAgo, cpu, null, null, 50, 60);

    private static TrendSeries Series(TimeSpan receivedAt, params float[] secsAgo) =>
        TrendSeries.From(new Trend(300, secsAgo.Select(s => Point(s)).ToList()), receivedAt);

    [Fact]
    public void Points_are_placed_on_the_local_clock()
    {
        var t = Series(TimeSpan.FromSeconds(1000), 4.5f, 0.5f);

        Assert.Equal([TimeSpan.FromSeconds(995.5), TimeSpan.FromSeconds(999.5)], t.Samples.Select(s => s.At));
    }

    [Fact]
    public void The_same_sample_lands_in_the_same_place_whenever_it_is_read()
    {
        // 同一个采样点：第一次读取时距今 1 秒，晚 2.3 秒（请求慢了）再读时距今 3.3 秒。
        var first = Series(TimeSpan.FromSeconds(100), 1f);
        var later = Series(TimeSpan.FromSeconds(102.3), 3.3f);
        var now = TimeSpan.FromSeconds(103);

        Assert.Equal(first.X(first.Samples[0].At, now, 600), later.X(later.Samples[0].At, now, 600), 6);
    }

    [Fact]
    public void Display_lags_so_the_right_edge_always_has_data()
    {
        var t = Series(TimeSpan.FromSeconds(100), 0f);
        var now = TimeSpan.FromSeconds(100);

        // 刚采到的点在右边缘之外（稍后滑入），右边缘对应 4 秒前。
        Assert.True(t.X(t.Samples[0].At, now, 300) > 300);
        Assert.Equal(300, t.X(now - TrendSeries.DisplayDelay, now, 300), 6);
    }

    [Fact]
    public void Points_older_than_the_window_fall_off_the_left_edge_instead_of_piling_up()
    {
        var t = Series(TimeSpan.FromSeconds(1000), 310f, 305f);
        var now = TimeSpan.FromSeconds(1000);

        Assert.True(t.X(t.Samples[0].At, now, 300) < t.X(t.Samples[1].At, now, 300));
        Assert.True(t.X(t.Samples[1].At, now, 300) < 0);
    }

    [Fact]
    public void Hybrid_and_collecting_follow_the_data()
    {
        var hybrid = TrendSeries.From(new Trend(300, [new HistoryPoint(2, 30, 60, 10, 50, null)]), TimeSpan.Zero);

        Assert.True(hybrid.Hybrid);
        Assert.Equal(60f, hybrid.Latest!.Value.Load);
        Assert.True(hybrid.Collecting);
        Assert.False(Series(TimeSpan.FromSeconds(200), 120, 2).Collecting);
        Assert.False(Series(TimeSpan.FromSeconds(200), 2).Hybrid);
    }
}
