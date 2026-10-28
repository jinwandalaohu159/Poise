using System.Net;
using System.Net.Http.Headers;
using System.Text.Json;

namespace Poise.Core.Updates;

/// <summary>
/// 从 GitHub Releases 读取最新版本。只访问 api.github.com，不需要登录（仓库公开）。
/// </summary>
public static class GitHubReleases
{
    public const string Repository = "jinwandalaohu159/Poise";

    private static readonly Uri LatestUrl = new($"https://api.github.com/repos/{Repository}/releases/latest");

    /// <summary>安装程序的文件名前缀（完整名称为 Poise-Setup-x.y.z.exe）。</summary>
    private const string InstallerPrefix = "Poise-Setup-";

    /// <summary>最新的正式版本；还没有发布过时返回 null。网络出错时抛出 <see cref="HttpRequestException"/>。</summary>
    public static async Task<ReleaseInfo?> FetchLatestAsync(HttpClient http, Version? current, CancellationToken cancel)
    {
        using var request = new HttpRequestMessage(HttpMethod.Get, LatestUrl);
        // GitHub 要求带 User-Agent。
        request.Headers.UserAgent.Add(new ProductInfoHeaderValue("Poise", current?.ToString(3) ?? "dev"));
        request.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/vnd.github+json"));
        using var response = await http.SendAsync(request, cancel);
        if (response.StatusCode == HttpStatusCode.NotFound)
        {
            return null;
        }
        response.EnsureSuccessStatusCode();
        return ParseLatest(await response.Content.ReadAsStringAsync(cancel));
    }

    /// <summary>
    /// 解析"最新版本"接口的返回。草稿、预发布、版本号无法识别、没有与版本号一致的安装程序、
    /// 安装程序没有 SHA-256 校验值的版本都返回 null（宁可不更新，也不安装无法校验的文件）。
    /// </summary>
    public static ReleaseInfo? ParseLatest(string json)
    {
        try
        {
            using var doc = JsonDocument.Parse(json);
            var root = doc.RootElement;
            if (Bool(root, "draft") || Bool(root, "prerelease"))
            {
                return null;
            }
            var tag = root.GetProperty("tag_name").GetString() ?? "";
            if (AppVersion.Parse(tag) is not { } version)
            {
                return null;
            }
            if (Installer(root, version) is not { } asset
                || !Uri.TryCreate(String(asset, "browser_download_url"), UriKind.Absolute, out var url)
                || Sha256(String(asset, "digest")) is not { } sha256)
            {
                return null;
            }
            var page = Uri.TryCreate(String(root, "html_url"), UriKind.Absolute, out var p)
                ? p
                : new Uri($"https://github.com/{Repository}/releases/tag/{tag}");
            return new ReleaseInfo(
                version,
                tag,
                String(root, "body") ?? "",
                page,
                url,
                asset.GetProperty("size").GetInt64(),
                sha256);
        }
        catch (Exception e) when (e is JsonException or KeyNotFoundException or InvalidOperationException or FormatException)
        {
            return null;
        }
    }

    /// <summary>与版本号一致的安装程序（Poise-Setup-x.y.z.exe）。名称对不上时不认，避免装到别的版本。</summary>
    private static JsonElement? Installer(JsonElement root, Version version)
    {
        if (!root.TryGetProperty("assets", out var assets) || assets.ValueKind != JsonValueKind.Array)
        {
            return null;
        }
        var exact = $"{InstallerPrefix}{version.ToString(3)}.exe";
        foreach (var a in assets.EnumerateArray())
        {
            if (string.Equals(String(a, "name"), exact, StringComparison.OrdinalIgnoreCase))
            {
                return a;
            }
        }
        return null;
    }

    /// <summary>"sha256:abc…" → "abc…"；其他算法或格式不对时返回 null。</summary>
    internal static string? Sha256(string? digest)
    {
        const string prefix = "sha256:";
        if (digest is null || !digest.StartsWith(prefix, StringComparison.OrdinalIgnoreCase))
        {
            return null;
        }
        var hex = digest[prefix.Length..].ToLowerInvariant();
        return hex.Length == 64 && hex.All(Uri.IsHexDigit) ? hex : null;
    }

    private static bool Bool(JsonElement e, string name) =>
        e.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.True;

    private static string? String(JsonElement e, string name) =>
        e.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;
}
