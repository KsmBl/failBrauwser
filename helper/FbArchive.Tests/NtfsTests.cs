namespace FbArchive.Tests;

/// <summary>NTFS images keep folders, as failBrauwser changed the vendored NTFS code.</summary>
[TestFixture]
public class NtfsTests {

  [Test]
  public void FoldersSurviveCreateAddMkdirAndRemove() {
    var root = Directory.CreateTempSubdirectory("fb-ntfs-");
    try {
      Directory.CreateDirectory(Path.Combine(root.FullName, "pack", "sub", "empty"));
      File.WriteAllText(Path.Combine(root.FullName, "pack", "sub", "Long Name.txt"), "deep");
      File.WriteAllBytes(Path.Combine(root.FullName, "pack", "big.bin"), Enumerable.Range(0, 20000).Select(i => (byte)i).ToArray());
      File.WriteAllText(Path.Combine(root.FullName, "top.txt"), "top");
      var img = Path.Combine(root.FullName, "n.ntfs");
      Commands.Add(img, [new(Path.Combine(root.FullName, "pack"), "pack"), new(Path.Combine(root.FullName, "top.txt"), "top.txt")], null, "Ntfs");
      Commands.Mkdir(img, "later/deeper", null);
      Commands.Add(img, [new(Path.Combine(root.FullName, "top.txt"), "pack/sub/added.txt")], null);
      Commands.Add(img, [new(Path.Combine(root.FullName, "top.txt"), "root2.txt")], null);
      Commands.Remove(img, ["top.txt"], null);
      Commands.Remove(img, ["pack/sub/empty"], null);

      var names = Compression.Lib.ArchiveOperations.List(img, null).Select(e => Commands.Normalize(e.Name)).ToList();
      Assert.That(names, Is.SupersetOf(new[] { "pack", "pack/sub", "pack/sub/Long Name.txt", "pack/sub/added.txt", "pack/big.bin", "later", "later/deeper", "root2.txt" }));
      Assert.That(names, Does.Not.Contain("top.txt").And.Not.Contain("pack/sub/empty"));
      var x = Path.Combine(root.FullName, "x");
      Commands.Extract(img, x, null, null);
      Assert.That(File.ReadAllText(Path.Combine(x, "pack", "sub", "Long Name.txt")), Is.EqualTo("deep"));
      Assert.That(File.ReadAllBytes(Path.Combine(x, "pack", "big.bin")), Is.EqualTo(File.ReadAllBytes(Path.Combine(root.FullName, "pack", "big.bin"))));
      Assert.That(Directory.Exists(Path.Combine(x, "later", "deeper")), Is.True);

      // Removing a folder takes its contents along.
      Commands.Remove(img, ["pack"], null);
      names = Compression.Lib.ArchiveOperations.List(img, null).Select(e => Commands.Normalize(e.Name)).ToList();
      Assert.That(names.Where(n => n == "pack" || n.StartsWith("pack/")), Is.Empty);
      Assert.That(names, Does.Contain("root2.txt"));
    }
    finally {
      root.Delete(true);
    }
  }

  /// <summary>The block sizes (from the update-sequence count) of every INDX block in the image.</summary>
  static HashSet<int> IndxBlockSizes(string img) {
    var bytes = File.ReadAllBytes(img);
    var sizes = new HashSet<int>();
    for (var off = 0; off + 8 <= bytes.Length; off += 512)
      if (bytes[off] == 'I' && bytes[off + 1] == 'N' && bytes[off + 2] == 'D' && bytes[off + 3] == 'X' && BitConverter.ToUInt16(bytes, off + 4) == 40)
        sizes.Add((BitConverter.ToUInt16(bytes, off + 6) - 1) * 512);
    return sizes;
  }

  /// <summary>
  /// The file-attribute flags of every $FILE_NAME of <paramref name="name"/> in the image:
  /// the one in its record and the copies in index keys.
  /// </summary>
  static List<uint> FileNameFlags(string img, string name) {
    var bytes = File.ReadAllBytes(img);
    var pattern = new byte[] { (byte)name.Length }.Concat(new byte[] { 0 }).ToArray(); // length, namespace (any)
    var utf16 = System.Text.Encoding.Unicode.GetBytes(name);
    var flags = new List<uint>();
    for (var i = 66; i + 2 + utf16.Length <= bytes.Length; i++) {
      if (bytes[i] != pattern[0] || !bytes.AsSpan(i + 2, utf16.Length).SequenceEqual(utf16)) continue;
      flags.Add(BitConverter.ToUInt32(bytes, i - 8)); // $FILE_NAME: flags @56, name length @64
    }
    return flags;
  }

  [Test]
  public void BigFoldersKeepTheBootSectorsIndexBlockSize() {
    // A folder of long names needs several levels of index blocks; a single long name
    // used to grow a folder's blocks to 64 KiB, which Linux cannot mount.
    var root = Directory.CreateTempSubdirectory("fb-ntfs-");
    try {
      var src = Path.Combine(root.FullName, "src");
      var many = Directory.CreateDirectory(Path.Combine(src, "long names")).FullName;
      for (var i = 0; i < 3000; i++)
        File.WriteAllText(Path.Combine(many, $"{i:D5} {new string('x', 194)}"), i.ToString());
      File.WriteAllText(Path.Combine(src, new string('n', 200) + ".txt"), "long");
      for (var i = 0; i < 8; i++)
        File.WriteAllText(Path.Combine(src, $"small{i}.txt"), $"small {i}");
      var img = Path.Combine(root.FullName, "big.ntfs");
      Commands.Add(img, [new(src, "src")], null, "Ntfs");

      Assert.That(IndxBlockSizes(img), Is.EquivalentTo(new[] { 4096 }));
      // The folder's key in its parent's index says it is a folder, like its record.
      Assert.That(FileNameFlags(img, "long names"), Has.Count.GreaterThanOrEqualTo(2).And.All.EqualTo(0x10000000u));
      var files = Compression.Lib.ArchiveOperations.List(img, null).Where(e => !e.IsDirectory).Select(e => Commands.Normalize(e.Name)).ToList();
      Assert.That(files.Count(n => n.StartsWith("src/long names/")), Is.EqualTo(3000));
      Assert.That(files, Does.Contain("src/" + new string('n', 200) + ".txt"));
      var x = Path.Combine(root.FullName, "x");
      Commands.Extract(img, x, null, null);
      Assert.That(File.ReadAllText(Path.Combine(x, "src", "long names", $"02999 {new string('x', 194)}")), Is.EqualTo("2999"));
      Assert.That(File.ReadAllText(Path.Combine(x, "src", "small7.txt")), Is.EqualTo("small 7"));
    }
    finally {
      root.Delete(true);
    }
  }

  [Test]
  public void TheJournalIsBigEnoughForLinuxToMountReadWrite() {
    // ntfs3 wants two restart pages and 48 record pages of 4 KiB in $LogFile (200 KiB);
    // a new image's log is never written to, so it is all 0xFF.
    var root = Directory.CreateTempSubdirectory("fb-ntfs-");
    try {
      var f = Path.Combine(root.FullName, "a.txt");
      File.WriteAllText(f, "a");
      var img = Path.Combine(root.FullName, "j.ntfs");
      Commands.Add(img, [new(f, "a.txt")], null, "Ntfs");
      var bytes = File.ReadAllBytes(img);
      int longest = 0, run = 0;
      foreach (var b in bytes) {
        run = b == 0xFF ? run + 1 : 0;
        longest = Math.Max(longest, run);
      }
      Assert.That(longest, Is.GreaterThanOrEqualTo(50 * 4096));
    }
    finally {
      root.Delete(true);
    }
  }

  [Test]
  public void AddingAndRemovingInTheRootKeepsTheIndexBlockSize() {
    // Files added one by one into the root until its index outgrows one block, then
    // most of them removed again: edits in place or laid out anew, never bigger blocks.
    var root = Directory.CreateTempSubdirectory("fb-ntfs-");
    try {
      var first = Path.Combine(root.FullName, "first.txt");
      File.WriteAllText(first, "first");
      Directory.CreateDirectory(Path.Combine(root.FullName, "kept folder"));
      var img = Path.Combine(root.FullName, "root.ntfs");
      Commands.Add(img, [new(first, "first.txt"), new(Path.Combine(root.FullName, "kept folder"), "kept folder")], null, "Ntfs");
      var name = (int i) => $"{i:D3} {new string('r', 150)}.txt";
      for (var i = 0; i < 40; i++) {
        var f = Path.Combine(root.FullName, "f.txt");
        File.WriteAllText(f, $"added {i}");
        Commands.Add(img, [new(f, name(i))], null);
      }
      Assert.That(IndxBlockSizes(img), Is.EquivalentTo(new[] { 4096 }));
      Assert.That(FileNameFlags(img, "kept folder"), Has.Count.GreaterThanOrEqualTo(2).And.All.EqualTo(0x10000000u));
      var names = Compression.Lib.ArchiveOperations.List(img, null).Select(e => Commands.Normalize(e.Name)).ToList();
      Assert.That(names, Is.SupersetOf(Enumerable.Range(0, 40).Select(name).Append("first.txt")));

      Commands.Remove(img, Enumerable.Range(0, 30).Select(name).ToArray(), null);
      Assert.That(IndxBlockSizes(img), Is.SubsetOf(new[] { 4096 }));
      names = Compression.Lib.ArchiveOperations.List(img, null).Select(e => Commands.Normalize(e.Name)).ToList();
      Assert.That(names, Is.SupersetOf(Enumerable.Range(30, 10).Select(name).Append("first.txt")));
      Assert.That(names.Intersect(Enumerable.Range(0, 30).Select(name)), Is.Empty);
      Assert.That(FileNameFlags(img, "kept folder"), Has.Count.GreaterThanOrEqualTo(2).And.All.EqualTo(0x10000000u));
      var x = Path.Combine(root.FullName, "x");
      Commands.Extract(img, x, null, null);
      Assert.That(File.ReadAllText(Path.Combine(x, name(39))), Is.EqualTo("added 39"));
    }
    finally {
      root.Delete(true);
    }
  }
}
