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
