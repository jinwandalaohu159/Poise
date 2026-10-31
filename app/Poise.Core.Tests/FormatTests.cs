namespace Poise.Core.Tests;

public sealed class FormatTests
{
    private static readonly TimeSpan Offset = TimeZoneInfo.Local.GetUtcOffset(new DateTime(2026, 10, 27, 15, 0, 0));
    private static readonly DateTimeOffset Now = new(2026, 10, 27, 15, 0, 0, Offset);

    [Theory]
    [InlineData(0UL, "0 MB")]
    [InlineData(512UL << 20, "512 MB")]
    [InlineData(1UL << 30, "1.0 GB")]
    [InlineData((2UL << 30) + (100UL << 20), "2.1 GB")]
    public void Bytes_show_mb_below_a_gigabyte_and_one_decimal_above(ulong bytes, string expected) =>
        Assert.Equal(expected, Format.Bytes(bytes));

    [Fact]
    public void Bytes_split_into_number_and_unit()
    {
        Assert.Equal(("2.1", "GB"), Format.BytesParts((2UL << 30) + (100UL << 20)));
        Assert.Equal(("512", "MB"), Format.BytesParts(512UL << 20));
    }

    [Theory]
    [InlineData(16UL << 30, "16 GB")]
    [InlineData((1UL << 30) + (512UL << 20), "1.5 GB")]
    [InlineData(780UL << 30, "780 GB")]
    [InlineData(2_000_398_934_016UL, "1.8 TB")]
    public void Capacities_drop_trailing_zeros_and_use_tb_above_a_terabyte(ulong bytes, string expected) =>
        Assert.Equal(expected, Format.Capacity(bytes));

    [Theory]
    [InlineData(820UL, "820")]
    [InlineData(8_200UL, "8,200")]
    [InlineData(46_000UL, "4.6 万")]
    [InlineData(1_680_000UL, "168 万")]
    public void Counts_use_wan_above_ten_thousand_in_chinese(ulong n, string expected)
    {
        TestSupport.UseChinese();

        Assert.Equal(expected, Format.Count(n));
    }

    [Fact]
    public void Durations_keep_the_two_largest_units()
    {
        TestSupport.UseChinese();

        Assert.Equal("3 天 4 小时", Format.Duration(new TimeSpan(3, 4, 30, 0)));
        Assert.Equal("5 小时 12 分钟", Format.Duration(new TimeSpan(5, 12, 0)));
        Assert.Equal("8 分钟", Format.Duration(TimeSpan.FromMinutes(8.5)));
        Assert.Equal("1 分钟", Format.Duration(TimeSpan.FromSeconds(10)));
    }

    [Fact]
    public void Temperature_and_percent_are_whole_numbers()
    {
        Assert.Equal("72°", Format.Temperature(71.6f));
        Assert.Equal("45%", Format.Percent(44.6f));
    }

    [Fact]
    public void When_is_relative_for_today_and_yesterday()
    {
        TestSupport.UseChinese();

        Assert.Equal("今天 09:05", Format.When(new DateTimeOffset(2026, 10, 27, 9, 5, 0, Offset), Now));
        Assert.Equal("昨天 23:59", Format.When(new DateTimeOffset(2026, 10, 26, 23, 59, 0, Offset), Now));
        Assert.Equal("10月20日", Format.When(new DateTimeOffset(2026, 10, 20, 8, 0, 0, Offset), Now));
    }

    [Fact]
    public void When_follows_the_ui_language()
    {
        TestSupport.UseEnglish();

        Assert.Equal("today 09:05", Format.When(new DateTimeOffset(2026, 10, 27, 9, 5, 0, Offset), Now));
        Assert.Equal("10/20", Format.When(new DateTimeOffset(2026, 10, 20, 8, 0, 0, Offset), Now));
    }
}
