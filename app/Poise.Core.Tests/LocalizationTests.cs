using System.Text.RegularExpressions;
using Poise.Core.Localization;

namespace Poise.Core.Tests;

/// <summary>两种语言的文案表：键一致、占位符一致、符合文案规范（书面、中性）。</summary>
public sealed partial class LocalizationTests
{
    private static readonly Dictionary<string, string> Zh = TestSupport.StringTable(Loc.Chinese);
    private static readonly Dictionary<string, string> En = TestSupport.StringTable(Loc.English);

    /// <summary>允许为空的键（例如"立即生效"不需要提示）。</summary>
    private static readonly HashSet<string> MayBeEmpty = ["effect.immediately"];

    [Fact]
    public void Both_languages_have_the_same_keys()
    {
        Assert.Empty(Zh.Keys.Except(En.Keys));
        Assert.Empty(En.Keys.Except(Zh.Keys));
    }

    [Fact]
    public void Placeholders_match_between_languages()
    {
        foreach (var key in Zh.Keys)
        {
            Assert.True(Placeholders(Zh[key]).SetEquals(Placeholders(En[key])), $"{key} 的占位符不一致");
        }
    }

    [Fact]
    public void No_text_is_empty()
    {
        foreach (var (key, value) in Zh.Concat(En))
        {
            Assert.True(MayBeEmpty.Contains(key) || !string.IsNullOrWhiteSpace(value), $"{key} 为空");
        }
    }

    [Theory]
    [InlineData("！")]
    [InlineData("!")]
    [InlineData("你")]
    [InlineData("吧")]
    [InlineData("啦")]
    [InlineData("哦")]
    [InlineData("累")]
    public void Chinese_text_is_not_colloquial(string banned)
    {
        var offending = Zh.Where(p => p.Value.Contains(banned)).Select(p => p.Key).ToList();
        Assert.True(offending.Count == 0, $"含有「{banned}」：{string.Join(", ", offending)}");
    }

    [Theory]
    [InlineData("!")]
    [InlineData("tired")]
    [InlineData("feeling")]
    public void English_text_is_not_colloquial(string banned)
    {
        var offending = En.Where(p => p.Value.Contains(banned, StringComparison.OrdinalIgnoreCase)).Select(p => p.Key).ToList();
        Assert.True(offending.Count == 0, $"contains \"{banned}\": {string.Join(", ", offending)}");
    }

    [Fact]
    public void Embedded_tables_are_loaded()
    {
        TestSupport.UseEnglish();
        Assert.Equal(En["home.clean"], Loc.Get("home.clean"));
        TestSupport.UseChinese();
        Assert.Equal(Zh["home.clean"], Loc.Get("home.clean"));
    }

    [Fact]
    public void Unknown_key_falls_back_to_the_key_itself()
    {
        TestSupport.UseChinese();
        Assert.Equal("no.such.key", Loc.Get("no.such.key"));
    }

    private static HashSet<string> Placeholders(string text) =>
        PlaceholderPattern().Matches(text).Select(m => m.Groups[1].Value).ToHashSet();

    [GeneratedRegex(@"\{(\d+)[^}]*\}")]
    private static partial Regex PlaceholderPattern();
}
