namespace Poise.Core.Updates;

/// <summary>GitHub 上发布的一个版本，以及它的安装程序。</summary>
/// <param name="Sha256">安装程序的 SHA-256（小写十六进制）。没有校验值的版本不会被提供更新。</param>
public sealed record ReleaseInfo(
    Version Version,
    string Tag,
    string Notes,
    Uri PageUrl,
    Uri InstallerUrl,
    long InstallerSize,
    string Sha256);
