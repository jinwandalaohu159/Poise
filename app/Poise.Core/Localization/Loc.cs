using System.Globalization;
using System.Reflection;
using System.Text.Json;

namespace Poise.Core.Localization;

/// <summary>
/// 中英文字符串表。语言切换后触发 <see cref="Changed"/>，界面据此重建当前页面。
/// </summary>
public static class Loc
{
    public const string Chinese = "zh-CN";
    public const string English = "en-US";

    private static readonly Dictionary<string, string> Fallback = Load(Chinese);
    private static Dictionary<string, string> _table = Fallback;

    public static string Language { get; private set; } = Chinese;

    public static event Action? Changed;

    /// <summary>设置语言；null 表示跟随系统。</summary>
    public static void SetLanguage(string? preferred)
    {
        var language = preferred ?? (CultureInfo.CurrentUICulture.Name.StartsWith("zh", StringComparison.OrdinalIgnoreCase) ? Chinese : English);
        if (language == Language && _table.Count > 0)
        {
            return;
        }
        Language = language;
        _table = language == Chinese ? Fallback : Load(language);
        Changed?.Invoke();
    }

    public static string Get(string key) =>
        _table.TryGetValue(key, out var v) ? v : Fallback.TryGetValue(key, out var f) ? f : key;

    public static string Format(string key, params object?[] args) =>
        string.Format(CultureInfo.GetCultureInfo(Language), Get(key), args);

    private static Dictionary<string, string> Load(string language)
    {
        var name = $"Poise.Core.Localization.Strings.{language}.json";
        using var stream = Assembly.GetExecutingAssembly().GetManifestResourceStream(name);
        return stream is null
            ? []
            : JsonSerializer.Deserialize(stream, LocalJson.Default.DictionaryStringString) ?? [];
    }
}
