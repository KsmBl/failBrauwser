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

  /// <summary>
  /// Every format that holds files: archives, file system images, tar combinations, single-file
  /// compressors and encoding wrappers (audio, video and image containers are left out).
  /// </summary>
  public static void Formats(Utf8JsonWriter w) {
    w.WriteStartArray("formats");
    foreach (var d in FormatRegistry.All) {
      var kind = d.Category switch {
        FormatCategory.Archive => "archive",
        FormatCategory.CompoundTar => "tar",
        FormatCategory.Stream => "stream",
        FormatCategory.Wrapper => "wrapper",
        _ => null,
      };
      if (kind == null) continue;
      var caps = d.Capabilities;
      w.WriteStartObject();
      w.WriteString("id", d.Id);
      w.WriteString("name", d.DisplayName);
      w.WriteString("kind", kind == "archive" && FormatRegistry.FilesystemFormatIds.Contains(d.Id) ? "filesystem" : kind);
      w.WriteBoolean("create", caps.HasFlag(FormatCapabilities.CanCreate));
      w.WriteBoolean("modify", caps.HasFlag(FormatCapabilities.CanModify));
      w.WriteBoolean("password", caps.HasFlag(FormatCapabilities.SupportsPassword));
      w.WriteBoolean("dirs", caps.HasFlag(FormatCapabilities.SupportsDirectories));
      w.WriteBoolean("multi", caps.HasFlag(FormatCapabilities.SupportsMultipleEntries));
      w.WriteStartArray("ext");
      foreach (var e in d.CompoundExtensions.Concat(d.Extensions).Select(e => e.ToLowerInvariant()).Distinct()) w.WriteStringValue(e);
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
    w.WriteBoolean("writable", archive && IsWritable(format, path));
  }

  /// <summary>An archive is writable when its format can be modified in place or rebuilt.</summary>
  public static bool IsWritable(F format, string? path = null) {
    if (format is F.Unknown or F.Sfx) return false;
    // ISO images are edited by rebuilding them (IsoEdit): Rock Ridge and Joliet names and
    // El Torito boot entries survive, hybrid (USB-bootable) images do not and stay read-only.
    if (format is F.Iso) return path == null || IsoEdit.ReadOnlyReason(path) == null;
    // A compressed single file can be written again: its one file is recompressed.
    if (FormatDetector.IsStreamFormat(format)) return FormatRegistry.GetStreamOps(format.ToString()) != null;
    var ops = FormatRegistry.GetArchiveOps(format.ToString());
    return ops is IArchiveModifiable || ops is IArchiveCreatable;
  }

  public static void List(Utf8JsonWriter w, string archive, string? password) {
    var format = FormatDetector.Detect(archive);
    var entries = ArchiveOperations.List(archive, password);
    w.WriteString("format", format.ToString());
    w.WriteBoolean("writable", IsWritable(format, archive));
    if (format == F.Iso && IsoEdit.ReadOnlyReason(archive) is { } why) w.WriteString("readonly_reason", why);
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
  /// <remarks><paramref name="format"/> (a format id) picks the format of a new archive; without
  /// it the extension decides.</remarks>
  public static void Add(string archive, IReadOnlyList<AddItem> items, string? password, string? format = null) {
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
      if (format == null) {
        ArchiveOperations.Create(archive, WithParents(inputs), opts);
      } else {
        if (!Enum.TryParse<F>(format, out var f)) throw new ProtocolException($"unknown format '{format}'");
        ArchiveOperations.Create(archive, FormatDetector.IsStreamFormat(f) ? inputs : WithParents(inputs), opts, f);
      }
      return;
    }
    RequireWritable(archive);
    if (IsIso(archive)) {
      IsoEdit.Edit(archive, tmp => Place(tmp, inputs));
      return;
    }
    if (IsStream(archive, out var streamFormat)) {
      // The one file inside is replaced (saving it after editing); nothing can be added.
      var inside = ArchiveOperations.List(archive, null).Single().Name;
      var files = inputs.Where(i => !i.IsDirectory).ToList();
      if (files.Count != 1 || inputs.Count != 1 || Normalize(files[0].EntryName) != Normalize(inside))
        throw new InvalidOperationException(OneFileOnly(archive));
      ArchiveOperations.Create(archive, files, opts, streamFormat);
      return;
    }
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
    if (IsStream(archive, out _)) throw new InvalidOperationException(OneFileOnly(archive));
    var wanted = names.Select(Normalize).Where(n => n.Length > 0).ToArray();
    if (IsIso(archive)) {
      IsoEdit.Edit(archive, tmp => {
        foreach (var n in wanted) {
          var p = Path.Combine(tmp, n);
          if (File.Exists(p)) File.Delete(p);
          else if (Directory.Exists(p)) Directory.Delete(p, true);
        }
      });
      return;
    }
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
    if (IsStream(archive, out _)) throw new InvalidOperationException($"The file inside “{Path.GetFileName(archive)}” is named after it: rename “{Path.GetFileName(archive)}” instead.");

    var entries = ArchiveOperations.List(archive, password);
    var names = entries.Select(e => Normalize(e.Name)).ToHashSet(StringComparer.Ordinal);
    if (names.Any(n => IsAtOrBelow(n, dst))) throw new IOException($"'{dst}' already exists in the archive.");
    var affected = entries.Where(e => IsAtOrBelow(Normalize(e.Name), src)).ToList();
    if (affected.Count == 0) throw new FileNotFoundException($"No such entry in archive: {from}");
    if (IsIso(archive)) {
      IsoEdit.Edit(archive, tmp => {
        var a = Path.Combine(tmp, src);
        var b = Path.Combine(tmp, dst);
        Directory.CreateDirectory(Path.GetDirectoryName(b)!);
        if (Directory.Exists(a)) Directory.Move(a, b); else File.Move(a, b);
      });
      return;
    }

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
    if (format == F.Iso && IsoEdit.ReadOnlyReason(archive) is { } why) throw new NotSupportedException(why);
    if (!IsWritable(format, archive)) throw new NotSupportedException($"{format} archives are read-only.");
  }

  /// <summary>True for a single-file compressor or encoding (.gz, .xz, .uue, …).</summary>
  private static bool IsStream(string archive, out F format) {
    format = File.Exists(archive) ? FormatDetector.Detect(archive) : F.Unknown;
    return FormatDetector.IsStreamFormat(format);
  }

  private static string OneFileOnly(string archive)
    => $"“{Path.GetFileName(archive)}” is one compressed file: files can be opened, edited and saved, not added or removed.";

  private static bool IsIso(string archive) => File.Exists(archive) && FormatDetector.Detect(archive) == F.Iso;

  /// <summary>Puts inputs into an extracted tree (for rebuilds).</summary>
  private static void Place(string tmp, IEnumerable<ArchiveInput> inputs) {
    foreach (var i in inputs) {
      var dest = Path.Combine(tmp, Normalize(i.EntryName));
      if (i.IsDirectory) {
        Directory.CreateDirectory(dest);
        if (!string.IsNullOrEmpty(i.FullPath) && Directory.Exists(i.FullPath)) Directory.SetLastWriteTimeUtc(dest, Directory.GetLastWriteTimeUtc(i.FullPath));
        continue;
      }
      Directory.CreateDirectory(Path.GetDirectoryName(dest)!);
      File.Copy(i.FullPath, dest, overwrite: true);
      File.SetLastWriteTimeUtc(dest, File.GetLastWriteTimeUtc(i.FullPath));
    }
  }
}
