using FileSystem.Iso;

namespace FbArchive;

/// <summary>
/// Editing ISO 9660 images by rebuilding them: extract (keeping Rock Ridge modes and times),
/// change, write a new image with Rock Ridge and Joliet names, and carry El Torito boot
/// entries over — boot images move with their files, isolinux boot info tables are patched
/// for the new position. Hybrid images (USB-bootable through a partition table with fixed
/// sector numbers) cannot survive that and stay read-only.
/// </summary>
public static class IsoEdit {

  /// <summary>Why this image cannot be edited, or null when it can.</summary>
  public static string? ReadOnlyReason(string path) {
    try {
      using var s = File.OpenRead(path);
      return IsoBoot.IsHybrid(s)
        ? "This image boots from USB through a partition table that a change would break."
        : null;
    } catch (IOException e) {
      return e.Message;
    }
  }

  /// <summary>Rebuilds <paramref name="archive"/> with the changes <paramref name="mutate"/> makes to the extracted tree.</summary>
  public static void Edit(string archive, Action<string> mutate) {
    if (ReadOnlyReason(archive) is { } why) throw new NotSupportedException(why);
    var full = Path.GetFullPath(archive);
    var dir = Path.GetDirectoryName(full)!;
    // Next to the image: same disk, so no second copy lands in a small /tmp.
    var temp = Directory.CreateDirectory(Path.Combine(dir, $".fb-iso-{Guid.NewGuid():N}"));
    var output = full + ".fb-new";
    try {
      var volumeId = "";
      (List<IsoBootEntry> Entries, uint CatalogLba)? boot;
      var boots = new List<(IsoBootEntry Entry, string? Path, byte[]? Data, bool Patch)>();
      string? catalogPath = null;
      using (var s = File.OpenRead(full)) {
        volumeId = IsoBoot.ReadVolumeId(s);
        boot = IsoBoot.ReadElTorito(s);
        s.Position = 0;
        using var reader = new IsoReader(s, leaveOpen: true);
        var total = reader.Entries.Where(e => !e.IsDirectory).Sum(e => e.Size);
        Progress.Current?.Phase("extracting", total, () => Progress.BytesBelow(temp.FullName));
        foreach (var e in reader.Entries.OrderBy(e => e.Name.Length)) {
          var target = Path.Combine(temp.FullName, Commands.Normalize(e.Name));
          if (e.IsDirectory) {
            Directory.CreateDirectory(target);
          } else {
            Directory.CreateDirectory(Path.GetDirectoryName(target)!);
            using (var f = File.Create(target)) reader.ExtractTo(e, f);
          }
          if (e.UnixMode is { } mode && !OperatingSystem.IsWindows()) {
            try { File.SetUnixFileMode(target, (UnixFileMode)(mode & 0xFFF)); } catch (IOException) { }
          }
          if (e.LastModified is { } t) {
            if (e.IsDirectory) Directory.SetLastWriteTimeUtc(target, t);
            else File.SetLastWriteTimeUtc(target, t);
          }
        }
        foreach (var be in boot?.Entries ?? []) {
          var file = reader.Entries.FirstOrDefault(e => !e.IsDirectory && e.FirstSector == be.LoadRba);
          var data = file != null ? reader.Extract(file) : IsoBoot.ReadAt(s, be.LoadRba, Math.Max(2048, be.SectorCount * 512));
          boots.Add((be, file != null ? Commands.Normalize(file.Name) : null, file == null ? data : null, IsoBoot.HasBootInfoTable(data, be.LoadRba)));
        }
        if (boot is { } b) {
          var cat = reader.Entries.FirstOrDefault(e => !e.IsDirectory && e.FirstSector == b.CatalogLba);
          if (cat != null) catalogPath = Commands.Normalize(cat.Name);
        }
      }

      mutate(temp.FullName);

      Progress.Current?.Phase("writing", 0);
      var writer = new IsoWriter { EnableRockRidge = true, EnableJoliet = true, VolumeIdentifier = volumeId.Length > 0 ? volumeId : "CDROM" };
      var bootFiles = boots.Where(x => x.Path != null).Select(x => x.Path!).ToHashSet(StringComparer.Ordinal);
      foreach (var d in Directory.EnumerateDirectories(temp.FullName, "*", SearchOption.AllDirectories)) {
        var rel = Path.GetRelativePath(temp.FullName, d).Replace('\\', '/');
        writer.AddDirectory(rel, ModeOf(d), Directory.GetLastWriteTimeUtc(d));
      }
      foreach (var f in Directory.EnumerateFiles(temp.FullName, "*", SearchOption.AllDirectories)) {
        var rel = Path.GetRelativePath(temp.FullName, f).Replace('\\', '/');
        var path = f;
        // Boot images are patched in memory; everything else streams from the disk.
        if (bootFiles.Contains(rel) || rel == catalogPath) writer.AddFile(rel, File.ReadAllBytes(path));
        else writer.AddStreamingFile(rel, new FileInfo(path).Length, () => File.OpenRead(path));
        writer.SetMetadata(rel, ModeOf(path), File.GetLastWriteTimeUtc(path));
      }
      foreach (var (entry, path, data, patch) in boots) {
        // A boot image the user deleted is gone from the boot menu too.
        if (path != null && !File.Exists(Path.Combine(temp.FullName, path))) continue;
        writer.BootEntries.Add(new IsoWriter.BootEntry {
          Platform = entry.Platform,
          MediaType = entry.MediaType,
          LoadSegment = entry.LoadSegment,
          SystemType = entry.SystemType,
          SectorCount = entry.SectorCount,
          ImagePath = path,
          ImageData = data,
          PatchBootInfoTable = patch,
        });
      }
      if (catalogPath != null && File.Exists(Path.Combine(temp.FullName, catalogPath))) writer.BootCatalogPath = catalogPath;

      using (var outStream = new FileStream(output, FileMode.Create, FileAccess.ReadWrite)) {
        writer.BuildToStreaming(outStream);
        outStream.Flush(flushToDisk: true);
      }
      File.Move(output, full, overwrite: true);
    } finally {
      try { temp.Delete(true); } catch (IOException) { }
      try { File.Delete(output); } catch (IOException) { }
    }
  }

  private static int? ModeOf(string path) {
    if (OperatingSystem.IsWindows()) return null;
    try { return (int)File.GetUnixFileMode(path); } catch (IOException) { return null; }
  }
}
