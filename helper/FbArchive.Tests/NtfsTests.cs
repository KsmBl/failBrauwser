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
}
