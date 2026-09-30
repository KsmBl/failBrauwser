using System.Diagnostics;

namespace FbArchive.Tests;

/// <summary>ext2/3/4 images keep folders, as failBrauwser changed the vendored ext code.</summary>
[TestFixture]
public class ExtTests {

  private static string? Tool(string name) =>
    new[] { "/usr/bin", "/usr/sbin", "/sbin", "/bin" }.Select(d => Path.Combine(d, name)).FirstOrDefault(File.Exists);

  private static string Run(string exe, params string[] args) {
    var psi = new ProcessStartInfo(exe) { RedirectStandardOutput = true, RedirectStandardError = true };
    foreach (var a in args) psi.ArgumentList.Add(a);
    using var p = Process.Start(psi)!;
    var output = p.StandardOutput.ReadToEnd() + p.StandardError.ReadToEnd();
    p.WaitForExit();
    return output;
  }

  [Test]
  public void FoldersSurviveCreateAddMkdirAndRemove() {
    var root = Directory.CreateTempSubdirectory("fb-ext-");
    try {
      Directory.CreateDirectory(Path.Combine(root.FullName, "pack", "sub", "empty"));
      File.WriteAllText(Path.Combine(root.FullName, "pack", "sub", "Long Name.txt"), "deep");
      File.WriteAllText(Path.Combine(root.FullName, "top.txt"), "top");
      var img = Path.Combine(root.FullName, "n.ext4");
      Commands.Add(img, [new(Path.Combine(root.FullName, "pack"), "pack"), new(Path.Combine(root.FullName, "top.txt"), "top.txt")], null, "Ext");
      Commands.Mkdir(img, "later/deeper", null);
      Commands.Add(img, [new(Path.Combine(root.FullName, "top.txt"), "pack/sub/added.txt")], null);
      Commands.Remove(img, ["top.txt"], null);

      var names = Compression.Lib.ArchiveOperations.List(img, null).Select(e => Commands.Normalize(e.Name)).ToList();
      Assert.That(names, Is.SupersetOf(new[] { "pack", "pack/sub", "pack/sub/empty", "pack/sub/Long Name.txt", "pack/sub/added.txt", "later", "later/deeper" }));
      Assert.That(names, Does.Not.Contain("top.txt"));
      var x = Path.Combine(root.FullName, "x");
      Commands.Extract(img, x, null, null);
      Assert.That(File.ReadAllText(Path.Combine(x, "pack", "sub", "Long Name.txt")), Is.EqualTo("deep"));
      Assert.That(Directory.Exists(Path.Combine(x, "later", "deeper")), Is.True);

      // The real tools agree, when they are installed.
      if (Tool("e2fsck") is { } fsck) Assert.That(Run(fsck, "-fn", img), Does.Not.Contain("Fix?").And.Not.Contain("ERROR"));
      if (Tool("debugfs") is { } debugfs) Assert.That(Run(debugfs, "-R", "ls /later", img), Does.Contain("deeper"));
    }
    finally {
      root.Delete(true);
    }
  }
}
