using Compression.Core.Streams;
using Compression.Lib;
using Compression.Registry;
using F = Compression.Lib.FormatDetector.Format;

namespace FbArchive;

/// <summary>
/// An archive cut into parts of equal size (<c>x.7z.001</c>, <c>x.7z.002</c>, …, as 7-Zip and
/// failBrauwser's Compress write them): opened from its first part as the archive inside, read
/// through all parts as one stream. Changing it would mean cutting it again: it is read-only.
/// </summary>
public static class SplitSet {
  public const string ReadOnlyReason = "A split archive (.001, .002, …) is opened read-only.";

  /// <summary>The parts, in order, when <paramref name="path"/> is the first of two or more.</summary>
  public static string[]? Parts(string path) {
    if (!path.EndsWith(".001", StringComparison.OrdinalIgnoreCase) || !File.Exists(path)) return null;
    var stem = path[..^4];
    var parts = new List<string> { path };
    for (var n = 2; n <= 999 && File.Exists($"{stem}.{n:D3}"); ++n) parts.Add($"{stem}.{n:D3}");
    return parts.Count > 1 ? parts.ToArray() : null;
  }

  /// <summary>The parts as one stream.</summary>
  public static Stream Open(string[] parts) => new ConcatenatedStream(parts.Select(p => (Stream)File.OpenRead(p)).ToArray());

  /// <summary>The format of the archive inside: by its contents, else by the name without ".001".</summary>
  public static F Format(string firstPart, Stream joined) {
    var header = new byte[Math.Min(64 * 1024, joined.Length)];
    joined.Position = 0;
    joined.ReadExactly(header);
    joined.Position = 0;
    var byMagic = FormatDetector.DetectByMagic(header);
    return byMagic != F.Unknown ? byMagic : FormatDetector.DetectByExtension(firstPart[..^4]);
  }

  /// <summary>The operations for the archive inside, or why it cannot be read.</summary>
  public static IArchiveFormatOperations Ops(F format)
    => FormatRegistry.GetArchiveOps(format.ToString()) ?? throw new NotSupportedException($"The split file holds a {format} file, which cannot be opened as a folder.");

  public static List<ArchiveEntry> List(string[] parts, string? password, out F format) {
    using var s = Open(parts);
    format = Format(parts[0], s);
    return Ops(format).List(s, password).Select(e =>
      new ArchiveEntry(e.Index, e.Name, e.OriginalSize, e.CompressedSize, e.Method, e.IsDirectory, e.IsEncrypted, e.LastModified)).ToList();
  }

  public static void Extract(string[] parts, string dest, string? password, string[]? names) {
    using var s = Open(parts);
    Ops(Format(parts[0], s)).Extract(s, dest, password, names);
  }
}
