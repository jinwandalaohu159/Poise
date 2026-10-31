using System.Collections.Concurrent;
using System.Globalization;

namespace Poise.Core.Presentation;

/// <summary>
/// 程序名的排序，与"开始"菜单、通讯录一致：按首字母分组，中文取拼音首字母，中英文混排
/// （百度网盘在 B，Chrome 在 C，钉钉和 docker 都在 D）；数字和符号开头的排在最前。
/// 同一字母内按中文拼音 / 英文字母顺序，不区分大小写。与界面语言无关。
/// <para>
/// 每个名称的首字母和排序键只算一次（见 <see cref="KeyOf"/>）：程序列表每两秒重新排一次，几百个名称
/// 每次比较都按中文规则现算的话，一次排序要做十几万次中文比较。
/// </para>
/// </summary>
public sealed class NameOrder : IComparer<string>
{
    public static readonly NameOrder Instance = new();

    private static readonly CompareInfo Chinese = CultureInfo.GetCultureInfo("zh-CN").CompareInfo;
    private const CompareOptions Options = CompareOptions.IgnoreCase | CompareOptions.IgnoreWidth;

    /// <summary>
    /// 拼音首字母的分界字（与 ICU 中文索引一致）：按拼音排序时，不早于某个分界字、又早于下一个分界字的汉字，首字母就是这个分界字对应的字母。
    /// 拼音没有 I、U、V 开头的音节。
    /// </summary>
    private static readonly (char Letter, string Boundary)[] Boundaries =
    [
        ('A', "阿"), ('B', "八"), ('C', "嚓"), ('D', "哒"), ('E', "妸"), ('F', "发"), ('G', "旮"),
        ('H', "哈"), ('J', "丌"), ('K', "咔"), ('L', "垃"), ('M', "嘸"), ('N', "拏"), ('O', "噢"),
        ('P', "妑"), ('Q', "七"), ('R', "呥"), ('S', "仨"), ('T', "他"), ('W', "屲"), ('X', "夕"),
        ('Y', "丫"), ('Z', "帀"),
    ];

    /// <summary>记住的名称最多这么多（程序装了又卸，名称不会无限增长，这只是兜底）。</summary>
    private const int MaxKeys = 4096;

    private static readonly ConcurrentDictionary<string, (char Initial, SortKey Key)> Keys = new();

    private NameOrder()
    {
    }

    public int Compare(string? x, string? y)
    {
        if (ReferenceEquals(x, y))
        {
            return 0;
        }
        if (x is null || y is null)
        {
            return x is null ? -1 : 1;
        }
        var (a, b) = (KeyOf(x), KeyOf(y));
        var byInitial = a.Initial.CompareTo(b.Initial);
        return byInitial != 0 ? byInitial : SortKey.Compare(a.Key, b.Key);
    }

    private static (char Initial, SortKey Key) KeyOf(string name)
    {
        if (Keys.TryGetValue(name, out var key))
        {
            return key;
        }
        if (Keys.Count >= MaxKeys)
        {
            Keys.Clear();
        }
        key = (Initial(name), Chinese.GetSortKey(name, Options));
        Keys.TryAdd(name, key);
        return key;
    }

    /// <summary>首字母（A～Z）；数字、符号或空名称为 '#'，排在字母前面。</summary>
    public static char Initial(string name)
    {
        var text = name.TrimStart();
        if (text.Length == 0)
        {
            return '#';
        }
        var first = text[0];
        if (char.IsAsciiLetter(first))
        {
            return char.ToUpperInvariant(first);
        }
        if (!IsHan(first))
        {
            return '#';
        }
        var ch = text[..1];
        var letter = '#';
        foreach (var (l, boundary) in Boundaries)
        {
            if (Chinese.Compare(ch, boundary, CompareOptions.None) < 0)
            {
                break;
            }
            letter = l;
        }
        return letter == '#' ? 'A' : letter;
    }

    /// <summary>常用汉字（中日韩统一表意文字基本区与扩展 A 区）。</summary>
    private static bool IsHan(char c) => c is >= '一' and <= '鿿' or >= '㐀' and <= '䶿';
}
