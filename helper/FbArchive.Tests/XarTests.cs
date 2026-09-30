namespace FbArchive.Tests;

/// <summary>XAR archives keep folders, as failBrauwser changed the vendored XAR code.</summary>
[TestFixture]
public class XarTests {

  [Test]
  public void FoldersSurviveCreateAddMkdirAndRemove() {
    var root = Directory.CreateTempSubdirectory("fb-xar-");
    try {
      Directory.CreateDirectory(Path.Combine(root.FullName, "pack", "sub", "empty"));
      File.WriteAllText(Path.Combine(root.FullName, "pack", "sub", "b.txt"), "deep");
      File.WriteAllText(Path.Combine(root.FullName, "top.txt"), "top");
      var xar = Path.Combine(root.FullName, "n.xar");
      Commands.Add(xar, [new(Path.Combine(root.FullName, "pack"), "pack"), new(Path.Combine(root.FullName, "top.txt"), "top.txt")], null, "Xar");
      Commands.Mkdir(xar, "later/deeper", null);
      Commands.Add(xar, [new(Path.Combine(root.FullName, "top.txt"), "pack/sub/added.txt")], null);
      Commands.Add(xar, [new(Path.Combine(root.FullName, "top.txt"), "root2.txt")], null);
      Commands.Remove(xar, ["top.txt"], null);
      Commands.Remove(xar, ["pack/sub/empty"], null);

      var names = Compression.Lib.ArchiveOperations.List(xar, null).Select(e => Commands.Normalize(e.Name)).ToList();
      Assert.That(names, Is.SupersetOf(new[] { "pack", "pack/sub", "pack/sub/b.txt", "pack/sub/added.txt", "later", "later/deeper", "root2.txt" }));
      Assert.That(names, Does.Not.Contain("top.txt").And.Not.Contain("pack/sub/empty"));
      var x = Path.Combine(root.FullName, "x");
      Commands.Extract(xar, x, null, null);
      Assert.That(File.ReadAllText(Path.Combine(x, "pack", "sub", "b.txt")), Is.EqualTo("deep"));
      Assert.That(File.ReadAllText(Path.Combine(x, "pack", "sub", "added.txt")), Is.EqualTo("top"));

      // Other tools accept it (libarchive checks the table-of-contents checksum).
      if (File.Exists("/usr/bin/bsdtar")) {
        var psi = new System.Diagnostics.ProcessStartInfo("/usr/bin/bsdtar", ["-tf", xar]) { RedirectStandardOutput = true, RedirectStandardError = true };
        using var p = System.Diagnostics.Process.Start(psi)!;
        var listed = p.StandardOutput.ReadToEnd() + p.StandardError.ReadToEnd();
        p.WaitForExit();
        Assert.That(p.ExitCode, Is.Zero, listed);
        Assert.That(listed, Does.Contain("pack/sub/added.txt"));
      }

      Commands.Remove(xar, ["pack"], null);
      names = Compression.Lib.ArchiveOperations.List(xar, null).Select(e => Commands.Normalize(e.Name)).ToList();
      Assert.That(names.Where(n => n == "pack" || n.StartsWith("pack/")), Is.Empty);
    }
    finally {
      root.Delete(true);
    }
  }
}
