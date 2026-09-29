namespace Compression.Registry;

/// <summary>
/// The edits an extract-mutate-recreate rebuild applies to its staging directory. Every
/// descriptor whose Add or Remove goes through a rebuild shares these, so an entry name
/// means the same thing in all of them.
/// </summary>
public static class RebuildStaging {

  /// <summary>
  /// The staging-relative form of an entry name: forward slashes, no leading <c>/</c> or
  /// <c>./</c>, no trailing slash. A filesystem image names its entries from the volume
  /// root ("/HELLO.TXT"), an archive does not; both land on the same staged path.
  /// </summary>
  public static string Normalize(string entryName) {
    var s = entryName.Replace('\\', '/');
    while (true) {
      if (s.StartsWith("./", StringComparison.Ordinal)) s = s[2..];
      else if (s.StartsWith('/')) s = s[1..];
      else break;
    }
    while (s.Contains("//", StringComparison.Ordinal)) s = s.Replace("//", "/", StringComparison.Ordinal);
    return s.TrimEnd('/');
  }

  /// <summary>
  /// Deletes the named entries from a staging tree. A name matches only the entry at exactly
  /// that path; a directory name removes the directory with everything below it.
  /// </summary>
  /// <remarks>
  /// Names used to match on their leaf as well, so removing <c>a.txt</c> also deleted
  /// <c>sub/a.txt</c>, and a directory entry was never removed at all — its files went, the
  /// empty folder stayed in the rebuilt archive. Paths on Unix are case-sensitive, so the
  /// comparison is ordinal: <c>A.txt</c> and <c>a.txt</c> are two different entries.
  /// </remarks>
  public static void RemoveEntries(string stagingDir, IEnumerable<string> entryNames) {
    ArgumentNullException.ThrowIfNull(stagingDir);
    foreach (var raw in entryNames ?? []) {
      var name = Normalize(raw);
      if (name.Length == 0 || name.Split('/').Any(p => p == "..")) continue;
      var path = Path.Combine(stagingDir, name.Replace('/', Path.DirectorySeparatorChar));
      if (File.Exists(path)) File.Delete(path);
      else if (Directory.Exists(path)) Directory.Delete(path, recursive: true);
    }
  }

  /// <summary>
  /// Places the inputs into a staging tree: files are written (replacing a staged file of the
  /// same name) and directory inputs become directories, so an empty folder survives the rebuild.
  /// </summary>
  public static void AddEntries(string stagingDir, IEnumerable<ArchiveInputInfo> inputs) {
    ArgumentNullException.ThrowIfNull(stagingDir);
    foreach (var input in inputs ?? []) {
      var name = Normalize(input.ArchiveName ?? "");
      if (name.Length == 0 || name.Split('/').Any(p => p == "..")) continue;
      var dest = Path.Combine(stagingDir, name.Replace('/', Path.DirectorySeparatorChar));
      if (input.IsDirectory) {
        Directory.CreateDirectory(dest);
        continue;
      }
      var destDir = Path.GetDirectoryName(dest);
      if (!string.IsNullOrEmpty(destDir)) Directory.CreateDirectory(destDir);
      if (input.InMemoryContent == null && File.Exists(input.FullPath)) {
        File.Copy(input.FullPath, dest, overwrite: true);
      } else {
        File.WriteAllBytes(dest, input.ReadContent());
      }
      if (input.SourceLastModifiedUtc is { } mtime) File.SetLastWriteTimeUtc(dest, mtime);
    }
  }
}
