using Compression.Lib;

namespace FbArchive.Tests;

/// <summary>Fixes to single formats of the vendored library that the format matrix found.</summary>
[TestFixture]
public class FormatFixesTests {

  private string _root = "";

  [SetUp]
  public void MakeRoot() => _root = Directory.CreateTempSubdirectory("fb-fix-").FullName;

  [TearDown]
  public void DropRoot() => Directory.Delete(_root, true);

  private string File(string name, string text) {
    var p = Path.Combine(_root, name);
    System.IO.File.WriteAllText(p, text);
    return p;
  }

  private static List<string> Names(string archive) => ArchiveOperations.List(archive, null).Where(e => !e.IsDirectory).Select(e => Commands.Normalize(e.Name)).ToList();

  /// <summary>BBC DFS names are "D.NAME": removing C.TXT used to delete A.TXT as well.</summary>
  [Test]
  public void Bbc_TheDirectoryLetterTellsFilesApart() {
    var ssd = Path.Combine(_root, "d.ssd");
    Commands.Add(ssd, [new(File("a.txt", "alpha"), "a.txt"), new(File("b.txt", "beta"), "b.txt")], null, "Bbc");
    Commands.Add(ssd, [new(File("c.txt", "gamma"), "c.txt")], null);
    Assert.That(Names(ssd), Is.EquivalentTo(new[] { "a.TXT", "b.TXT", "c.TXT" }));
    Commands.Remove(ssd, ["c.TXT"], null);
    Assert.That(Names(ssd), Is.EquivalentTo(new[] { "a.TXT", "b.TXT" }));
    // Extracted under the listed name.
    var x = Path.Combine(_root, "x");
    Commands.Extract(ssd, x, ["a.TXT"], null);
    Assert.That(System.IO.File.ReadAllText(Path.Combine(x, "a.TXT")), Is.EqualTo("alpha"));
  }
}
