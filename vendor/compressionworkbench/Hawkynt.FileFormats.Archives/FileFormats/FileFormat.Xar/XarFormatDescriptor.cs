#pragma warning disable CS1591
using Compression.Registry;
using static Compression.Registry.FormatHelpers;

namespace FileFormat.Xar;

/// <summary>
/// eXtensible ARchive (XAR) — gzip-compressed XML table of contents + heap; used by Apple installer packages.
///
/// References:
/// <list type="bullet">
///   <item><description><c>https://github.com/mackyle/xar</c> — maintained xar sources (format documentation in the repository)</description></item>
///   <item><description><c>https://en.wikipedia.org/wiki/Xar_(archiver)</c> — Wikipedia overview</description></item>
///   <item><description>originally released as an OpenDarwin/Apple open-source project</description></item>
/// </list>
/// </summary>
public sealed class XarFormatDescriptor : IFormatDescriptor, IArchiveFormatOperations, IArchiveCreatable, IArchiveModifiable, IArchiveDefragmentable, IArchiveLayoutMap {

  /// <summary>Rebuild-based defrag: extracts then re-creates the XAR archive in listing order.</summary>
  public void Defragment(Stream archive)
    => this.Defragment(archive, new DefragOptions { Mode = DefragMode.ConsolidateAtStart });

  /// <summary>Rebuild-based defrag: extracts then re-creates the XAR archive per the requested mode.</summary>
  public void Defragment(Stream archive, DefragOptions options) {
    DefragRebuilder.Rebuild(archive, options,
      readEntries: stream => {
        var r = new XarReader(stream);
        return r.Entries.Where(e => !e.IsDirectory).Select(e => (e.FileName, r.Extract(e)));
      },
      buildImage: files => {
        using var ms = new MemoryStream();
        using (var w = new XarWriter(ms)) {
          foreach (var (n, d) in files) w.AddFile(n, d);
        }
        return ms.ToArray();
      });
  }


  /// <inheritdoc />
  public IEnumerable<DefragBlockInfo> EnumerateLayout(Stream archive) {
    archive.Position = 0;
    var r = new XarReader(archive);
    foreach (var e in r.Entries) {
      if (e.CompressedSize > 0) {
        var absOffset = r.HeapStart + e.HeapOffset;
        yield return new DefragBlockInfo(absOffset, e.CompressedSize, DefragBlockKind.Used, FileName: e.FileName);
      }
    }
    // XAR header + TOC region
    if (r.HeapStart > 0)
      yield return new DefragBlockInfo(0, r.HeapStart, DefragBlockKind.MetadataReserved, FileName: "XAR Header + TOC");
  }

  /// <summary>
  /// Gets the id.
  /// </summary>
  public string Id => "Xar";
  /// <summary>
  /// Gets the display name.
  /// </summary>
  public string DisplayName => "XAR";
  /// <summary>
  /// Gets the category.
  /// </summary>
  public FormatCategory Category => FormatCategory.Archive;
  /// <summary>
  /// Gets the capabilities.
  /// </summary>
  public FormatCapabilities Capabilities =>
    FormatCapabilities.CanList | FormatCapabilities.CanExtract | FormatCapabilities.CanCreate |
    FormatCapabilities.CanModify | FormatCapabilities.CanTest |
    FormatCapabilities.SupportsMultipleEntries | FormatCapabilities.SupportsDirectories;
  /// <summary>
  /// Gets the default extension.
  /// </summary>
  public string DefaultExtension => ".xar";
  /// <summary>
  /// Gets the extensions.
  /// </summary>
  public IReadOnlyList<string> Extensions => [".xar"];
  /// <summary>
  /// Gets the compound extensions.
  /// </summary>
  public IReadOnlyList<string> CompoundExtensions => [];
  /// <summary>
  /// Gets the magic signatures.
  /// </summary>
  public IReadOnlyList<MagicSignature> MagicSignatures => [new([(byte)'x', (byte)'a', (byte)'r', (byte)'!'], Confidence: 0.95)];
  /// <summary>
  /// Gets the methods.
  /// </summary>
  public IReadOnlyList<FormatMethodInfo> Methods => [new("xar", "XAR")];
  /// <summary>
  /// Gets the tar compression format id.
  /// </summary>
  public string? TarCompressionFormatId => null;
  /// <summary>
  /// Gets the family.
  /// </summary>
  public AlgorithmFamily Family => AlgorithmFamily.Archive;
  /// <summary>
  /// Gets the description.
  /// </summary>
  public string Description => "eXtensible ARchive format (Apple pkg)";

  /// <summary>
  /// Lists the entries in the supplied container.
  /// </summary>
  public List<ArchiveEntryInfo> List(Stream stream, string? password) {
    var r = new XarReader(stream);
    return r.Entries.Select((e, i) => new ArchiveEntryInfo(i, e.FileName, e.OriginalSize, e.CompressedSize,
      e.Method, e.IsDirectory, false, e.LastModified)).ToList();
  }

  /// <summary>
  /// Decodes the supplied input.
  /// </summary>
  public void Extract(Stream stream, string outputDir, string? password, string[]? files) {
    var r = new XarReader(stream);
    foreach (var e in r.Entries) {
      if (e.IsDirectory) continue;
      if (files != null && !MatchesFilter(e.FileName, files)) continue;
      WriteFile(outputDir, e.FileName, r.Extract(e));
    }
  }

  /// <summary>
  /// Performs the create operation.
  /// </summary>
  public void Create(Stream output, IReadOnlyList<ArchiveInputInfo> inputs, FormatCreateOptions options) {
    // leaveOpen: true — caller owns the stream (e.g. AtomicFileWriter flushes
    // it to disk after we return; closing it here would break that contract).
    using var w = new XarWriter(output, leaveOpen: true);
    foreach (var i in inputs.Where(i => i.IsDirectory))
      w.AddDirectory(i.ArchiveName);
    foreach (var (name, data) in FormatHelpers.FilesOnly(inputs))
      w.AddFile(name, data);
  }

  /// <summary>
  /// Writes the archive anew: what is not dropped (a name or a folder above it), then the
  /// new folders and files. Used for edits below the top level, which the in-place
  /// modifier does not handle.
  /// </summary>
  private static void Rebuild(Stream archive, IReadOnlyList<ArchiveInputInfo> additions, ISet<string> drop) {
    archive.Position = 0;
    var r = new XarReader(archive);
    var dirs = new List<(string, DateTime?)>();
    var files = new List<(string, byte[], DateTime?)>();
    bool Dropped(string name) {
      var parts = name.Split('/');
      for (var n = 1; n <= parts.Length; ++n)
        if (drop.Contains(string.Join('/', parts[..n]))) return true;
      return false;
    }
    foreach (var e in r.Entries) {
      if (Dropped(e.FileName)) continue;
      if (e.IsDirectory) dirs.Add((e.FileName, e.LastModified));
      else files.Add((e.FileName, r.Extract(e), e.LastModified));
    }
    using var ms = new MemoryStream();
    using (var w = new XarWriter(ms, leaveOpen: true)) {
      foreach (var (name, modified) in dirs) w.AddDirectory(name, modified);
      foreach (var i in additions.Where(i => i.IsDirectory)) w.AddDirectory(i.ArchiveName);
      foreach (var (name, data, modified) in files) w.AddFile(name, data, modified);
      foreach (var (name, data) in FormatHelpers.FilesOnly(additions)) w.AddFile(name.Replace('\\', '/').Trim('/'), data);
    }
    archive.Position = 0;
    archive.SetLength(0);
    ms.Position = 0;
    ms.CopyTo(archive);
  }

  /// <summary>Adds (or replaces by name) files and folders inside an existing XAR archive.</summary>
  public void Add(Stream archive, IReadOnlyList<ArchiveInputInfo> inputs) {
    // Rewritten through the writer: the in-place modifier knows a flat table of contents
    // only and leaves the TOC checksum stale, which other tools then reject.
    var replaced = FormatHelpers.FilesOnly(inputs).Select(f => f.Name.Replace('\\', '/').Trim('/'));
    Rebuild(archive, inputs, new HashSet<string>(replaced, StringComparer.Ordinal));
  }

  /// <summary>Removes entries (folders with their contents) from an existing XAR archive.</summary>
  public void Remove(Stream archive, string[] entryNames) {
    // Rewritten through the writer, like Add.
    Rebuild(archive, [], new HashSet<string>(entryNames.Select(n => n.Replace('\\', '/').Trim('/')), StringComparer.Ordinal));
  }
}
