using System.Text.Json;
using Compression.Lib;
using Compression.Registry;
using F = Compression.Lib.FormatDetector.Format;

namespace FbArchive;

/// <summary>The operations failBrauwser needs to treat an archive like a directory.</summary>
public static class Commands {
  static Commands() => FormatRegistration.EnsureInitialized();

  // ── Names ───────────────────────────────────────────────────────────

  /// <summary>
  /// The canonical, relative, forward-slash form of an entry name, without a trailing slash.
  /// failBrauwser uses exactly this form for display and to build paths inside an archive.
  /// </summary>
  public static string Normalize(string raw) {
    var s = raw.Replace('\\', '/');
    while (true) {
      if (s.StartsWith("./", StringComparison.Ordinal)) s = s[2..];
      else if (s.StartsWith('/')) s = s[1..];
      else break;
    }
    while (s.Contains("//", StringComparison.Ordinal)) s = s.Replace("//", "/", StringComparison.Ordinal);
    return s.TrimEnd('/');
  }

  /// <summary>True when <paramref name="name"/> is <paramref name="dir"/> itself or lies below it.</summary>
  public static bool IsAtOrBelow(string name, string dir)
    => dir.Length == 0 || name == dir || (name.Length > dir.Length && name.StartsWith(dir, StringComparison.Ordinal) && name[dir.Length] == '/');

  // ── Queries ─────────────────────────────────────────────────────────

  public static void Formats(Utf8JsonWriter w) {
    w.WriteStartArray("formats");
    foreach (var d in FormatRegistry.All) {
      if (d.Category is not (FormatCategory.Archive or FormatCategory.CompoundTar)) continue;
      w.WriteStartObject();
      w.WriteString("id", d.Id);
      w.WriteString("name", d.DisplayName);
      w.WriteBoolean("create", d.Capabilities.HasFlag(FormatCapabilities.CanCreate));
      w.WriteBoolean("modify", d.Capabilities.HasFlag(FormatCapabilities.CanModify));
      w.WriteStartArray("ext");
      foreach (var e in d.CompoundExtensions) w.WriteStringValue(e);
      foreach (var e in d.Extensions) w.WriteStringValue(e);
      w.WriteEndArray();
      w.WriteEndObject();
    }
    w.WriteEndArray();
  }

  public static void Probe(Utf8JsonWriter w, string path) {
    var format = File.Exists(path) ? FormatDetector.Detect(path) : F.Unknown;
    var archive = format != F.Unknown && (FormatDetector.IsArchive(format) || FormatRegistry.GetArchiveOps(format.ToString()) != null);
    w.WriteBoolean("archive", archive);
    w.WriteString("format", format.ToString());
    w.WriteBoolean("writable", archive && IsWritable(format));
  }

  /// <summary>An archive is writable when its format can be modified in place or rebuilt.</summary>
  public static bool IsWritable(F format) {
    if (format is F.Unknown or F.Sfx) return false;
    // ISO images: the in-place modifier only updates the plain ISO 9660 tree (8.3 names,
    // flattened into the root), which Rock Ridge / Joliet readers never see, and a rebuild
    // would drop Rock Ridge metadata and El Torito boot records. Browse and extract only.
    if (format is F.Iso) return false;
    if (FormatDetector.IsStreamFormat(format)) return false;
    var ops = FormatRegistry.GetArchiveOps(format.ToString());
    return ops is IArchiveModifiable || ops is IArchiveCreatable;
  }

  public static void List(Utf8JsonWriter w, string archive, string? password) {
    var format = FormatDetector.Detect(archive);
    var entries = ArchiveOperations.List(archive, password);
    w.WriteString("format", format.ToString());
    w.WriteBoolean("writable", IsWritable(format));
    w.WriteStartArray("entries");
    foreach (var e in entries) {
      w.WriteStartObject();
      w.WriteString("raw", e.Name);
      w.WriteString("name", Normalize(e.Name));
      w.WriteBoolean("dir", e.IsDirectory || e.Name.EndsWith('/') || e.Name.EndsWith('\\'));
      w.WriteNumber("size", e.OriginalSize);
      w.WriteNumber("csize", e.CompressedSize);
      w.WriteBoolean("enc", e.IsEncrypted);
      w.WriteString("method", e.Method);
      if (e.LastModified is { } lm) {
        var utc = lm.Kind == DateTimeKind.Unspecified ? DateTime.SpecifyKind(lm, DateTimeKind.Local) : lm;
        try { w.WriteNumber("mtime", new DateTimeOffset(utc).ToUnixTimeSeconds()); }
        catch (ArgumentOutOfRangeException) { }
      }
      w.WriteEndObject();
    }
    w.WriteEndArray();
  }

  // ── Reading ─────────────────────────────────────────────────────────

  /// <summary>
  /// Extracts the named entries (normalized names; a directory brings everything below it) into
  /// <paramref name="dest"/>, keeping their paths relative to the archive root.
  /// A null list extracts everything.
  /// </summary>
  public static void Extract(string archive, string dest, string[]? names, string? password) {
    Directory.CreateDirectory(dest);
    string[]? raw = null;
    var all = ArchiveOperations.List(archive, password);
    // Progress: what has arrived in the destination out of what is coming.
    var before = Progress.BytesBelow(dest);
    var wantedSize = names == null
      ? all.Where(e => !e.IsDirectory).Sum(e => e.OriginalSize)
      : all.Where(e => !e.IsDirectory && names.Select(Normalize).Any(n => IsAtOrBelow(Normalize(e.Name), n))).Sum(e => e.OriginalSize);
    Progress.Current?.Phase("extracting", wantedSize, () => Progress.BytesBelow(dest) - before);
    if (names != null) {
      var wanted = names.Select(Normalize).ToArray();
      raw = all.Where(e => wanted.Any(n => IsAtOrBelow(Normalize(e.Name), n))).Select(e => e.Name).ToArray();
      if (raw.Length == 0) throw new FileNotFoundException($"No such entry in archive: {string.Join(", ", names)}");
      // Make sure empty directories that were asked for exist as well.
      foreach (var e in all.Where(e => e.IsDirectory && wanted.Any(n => IsAtOrBelow(Normalize(e.Name), n))))
        Directory.CreateDirectory(Path.Combine(dest, Normalize(e.Name)));
    }
    ArchiveOperations.Extract(archive, dest, password, raw);
  }

  // ── Writing ─────────────────────────────────────────────────────────

  /// <summary>
  /// Adds files and directories. A directory source is added recursively under its target name.
  /// A missing archive is created, in the format its extension names.
  /// </summary>
  public static void Add(string archive, IReadOnlyList<AddItem> items, string? password) {
    var inputs = new List<ArchiveInput>();
    foreach (var item in items) Expand(item, inputs);
    if (inputs.Count == 0) return;
    var opts = new CompressionOptions { Password = password };
    // Progress: bytes of the inputs read by the writer; a rebuild reads the old contents too.
    var inputBytes = inputs.Where(i => !i.IsDirectory && File.Exists(i.FullPath)).Sum(i => new FileInfo(i.FullPath).Length);
    var existing0 = File.Exists(archive) && new FileInfo(archive).Length > 0 && !(FormatRegistry.GetArchiveOps(FormatDetector.Detect(archive).ToString()) is IArchiveModifiable)
      ? ArchiveOperations.List(archive, password).Where(e => !e.IsDirectory).Sum(e => e.OriginalSize)
      : 0;
    Progress.Current?.Phase("adding", inputBytes + existing0);

    if (!File.Exists(archive) || new FileInfo(archive).Length == 0) {
      ArchiveOperations.Create(archive, WithParents(inputs), opts);
      return;
    }
    RequireWritable(archive);
    if (password != null) {
      // In-place edits of some formats (ZIP) ignore the password and would add new entries
      // unencrypted: with a password the archive is rebuilt, everything encrypted.
      RebuildEncrypted(archive, password, tmp => {
        foreach (var i in inputs) {
          var dest = Path.Combine(tmp, Normalize(i.EntryName));
          if (i.IsDirectory) { Directory.CreateDirectory(dest); continue; }
          Directory.CreateDirectory(Path.GetDirectoryName(dest)!);
          File.Copy(i.FullPath, dest, overwrite: true);
          File.SetLastWriteTimeUtc(dest, File.GetLastWriteTimeUtc(i.FullPath));
        }
      });
      return;
    }
    var existing = ArchiveOperations.List(archive, password).Select(e => Normalize(e.Name)).ToHashSet(StringComparer.Ordinal);
    // Replacing a file means dropping the old entry first, otherwise some formats keep both.
    var replaced = inputs.Where(i => !i.IsDirectory && existing.Contains(Normalize(i.EntryName))).Select(i => Normalize(i.EntryName)).ToArray();
    if (replaced.Length > 0) Remove(archive, replaced, password);
    var fresh = inputs.Where(i => !i.IsDirectory || !existing.Contains(Normalize(i.EntryName))).ToList();
    if (fresh.Count > 0) ArchiveOperations.Add(archive, fresh, opts);
  }

  private static void Expand(AddItem item, List<ArchiveInput> into) {
    var name = Normalize(item.Name);
    if (name.Length == 0) throw new ProtocolException("empty entry name");
    if (name.Split('/').Any(p => p is ".." or ".")) throw new ProtocolException($"invalid entry name '{item.Name}'");
    if (string.IsNullOrEmpty(item.Src)) {
      into.Add(new ArchiveInput("", name + "/"));
      return;
    }
    if (Directory.Exists(item.Src)) {
      // The source path rides along so the entry keeps the folder's timestamp.
      into.Add(new ArchiveInput(Path.GetFullPath(item.Src), name + "/"));
      var options = new EnumerationOptions { RecurseSubdirectories = true, IgnoreInaccessible = true, AttributesToSkip = 0 };
      foreach (var sub in Directory.EnumerateFileSystemEntries(item.Src, "*", options)) {
        var rel = name + "/" + Path.GetRelativePath(item.Src, sub).Replace('\\', '/');
        if (Directory.Exists(sub)) into.Add(new ArchiveInput(Path.GetFullPath(sub), rel + "/"));
        else into.Add(new ArchiveInput(Path.GetFullPath(sub), rel));
      }
      return;
    }
    if (!File.Exists(item.Src)) throw new FileNotFoundException("Source not found", item.Src);
    into.Add(new ArchiveInput(Path.GetFullPath(item.Src), name));
  }

  /// <summary>Adds the implied parent directory entries so a fresh archive has a complete tree.</summary>
  private static List<ArchiveInput> WithParents(List<ArchiveInput> inputs) {
    var seen = inputs.Where(i => i.IsDirectory).Select(i => Normalize(i.EntryName)).ToHashSet(StringComparer.Ordinal);
    var result = new List<ArchiveInput>();
    foreach (var i in inputs) {
      var parts = Normalize(i.EntryName).Split('/');
      for (var n = 1; n < parts.Length; n++) {
        var dir = string.Join('/', parts.Take(n));
        if (seen.Add(dir)) result.Add(new ArchiveInput("", dir + "/"));
      }
      result.Add(i);
    }
    return result;
  }

  /// <summary>Removes entries; a directory name removes the whole subtree.</summary>
  public static void Remove(string archive, string[] names, string? password) {
    RequireWritable(archive);
    var wanted = names.Select(Normalize).Where(n => n.Length > 0).ToArray();
    var listing = ArchiveOperations.List(archive, password);
    var raw = listing
      .Where(e => wanted.Any(n => IsAtOrBelow(Normalize(e.Name), n)))
      .Select(e => e.Name)
      .ToArray();
    if (raw.Length == 0) return;
    if (password != null) {
      RebuildEncrypted(archive, password, tmp => {
        foreach (var n in wanted) {
          var p = Path.Combine(tmp, n);
          if (File.Exists(p)) File.Delete(p);
          else if (Directory.Exists(p)) Directory.Delete(p, true);
        }
      });
      return;
    }
    // A rebuild re-reads what stays.
    Progress.Current?.Phase("removing", listing.Where(e => !e.IsDirectory && !raw.Contains(e.Name)).Sum(e => e.OriginalSize));
    ArchiveOperations.Remove(archive, raw, new CompressionOptions { Password = password });
  }

  /// <summary>Creates an empty folder, stamped with the current time.</summary>
  public static void Mkdir(string archive, string name, string? password) {
    var temp = Directory.CreateTempSubdirectory("fb-archive-mkdir-");
    try {
      var empty = Directory.CreateDirectory(Path.Combine(temp.FullName, "d"));
      Add(archive, [new AddItem(empty.FullName, name.TrimEnd('/'))], password);
    }
    finally {
      try { temp.Delete(true); } catch (IOException) { }
    }
  }

  /// <summary>Renames or moves an entry (and everything below it) inside the archive.</summary>
  public static void Rename(string archive, string from, string to, string? password) {
    RequireWritable(archive);
    var src = Normalize(from);
    var dst = Normalize(to);
    if (src.Length == 0 || dst.Length == 0) throw new ProtocolException("empty entry name");
    if (src == dst) return;
    if (IsAtOrBelow(dst, src)) throw new InvalidOperationException("Cannot move a folder into itself.");

    var entries = ArchiveOperations.List(archive, password);
    var names = entries.Select(e => Normalize(e.Name)).ToHashSet(StringComparer.Ordinal);
    if (names.Any(n => IsAtOrBelow(n, dst))) throw new IOException($"'{dst}' already exists in the archive.");
    var affected = entries.Where(e => IsAtOrBelow(Normalize(e.Name), src)).ToList();
    if (affected.Count == 0) throw new FileNotFoundException($"No such entry in archive: {from}");

    var temp = Directory.CreateTempSubdirectory("fb-archive-rename-");
    try {
      Extract(archive, temp.FullName, [src], password);
      // The extracted subtree is re-added as a whole; files keep the times extraction gave them.
      var staged = Path.Combine(temp.FullName, src);
      if (!File.Exists(staged) && !Directory.Exists(staged)) Directory.CreateDirectory(staged);
      var items = new List<AddItem> { new(staged, dst) };
      Remove(archive, [src], password);
      Add(archive, items, password);
    }
    finally {
      try { temp.Delete(true); } catch (IOException) { }
    }
  }

  /// <summary>
  /// Extracts everything with the password, lets <paramref name="mutate"/> change the tree,
  /// and writes the archive anew in the same format, encrypted with the same password.
  /// </summary>
  private static void RebuildEncrypted(string archive, string password, Action<string> mutate) {
    var format = FormatDetector.Detect(archive);
    var temp = Directory.CreateTempSubdirectory("fb-archive-crypt-");
    try {
      var all = ArchiveOperations.List(archive, password);
      Progress.Current?.Phase("removing", all.Where(e => !e.IsDirectory).Sum(e => e.OriginalSize));
      ArchiveOperations.Extract(archive, temp.FullName, password, null);
      foreach (var d in all.Where(e => e.IsDirectory)) Directory.CreateDirectory(Path.Combine(temp.FullName, Normalize(d.Name)));
      mutate(temp.FullName);
      var inputs = new List<ArchiveInput>();
      foreach (var dir in Directory.GetDirectories(temp.FullName, "*", SearchOption.AllDirectories))
        inputs.Add(new ArchiveInput(dir, Path.GetRelativePath(temp.FullName, dir).Replace('\\', '/') + "/"));
      foreach (var file in Directory.GetFiles(temp.FullName, "*", SearchOption.AllDirectories))
        inputs.Add(new ArchiveInput(file, Path.GetRelativePath(temp.FullName, file).Replace('\\', '/')));
      ArchiveOperations.Create(archive, inputs, new CompressionOptions { Password = password }, format);
    }
    finally {
      try { temp.Delete(true); } catch (IOException) { }
    }
  }

  private static void RequireWritable(string archive) {
    var format = FormatDetector.Detect(archive);
    if (!IsWritable(format)) throw new NotSupportedException($"{format} archives are read-only.");
  }
}
