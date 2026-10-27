using System.Text.Json;

namespace Poise.Core.Settings;

public enum ThemeChoice { System, Light, Dark }

/// <summary>只属于界面的本地设置（语言、外观），保存在用户目录。</summary>
public sealed class UiSettings
{
    private static readonly string FilePath = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Poise", "ui.json");

    /// <summary>null 表示跟随系统。</summary>
    public string? Language { get; set; }

    public ThemeChoice Theme { get; set; } = ThemeChoice.System;

    public static UiSettings Load()
    {
        try
        {
            return File.Exists(FilePath)
                ? JsonSerializer.Deserialize<UiSettings>(File.ReadAllText(FilePath)) ?? new UiSettings()
                : new UiSettings();
        }
        catch (Exception e) when (e is IOException or JsonException or UnauthorizedAccessException)
        {
            return new UiSettings();
        }
    }

    public void Save()
    {
        try
        {
            Directory.CreateDirectory(Path.GetDirectoryName(FilePath)!);
            File.WriteAllText(FilePath, JsonSerializer.Serialize(this));
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            // 保存失败不影响使用，下次启动回到默认设置。
        }
    }
}
