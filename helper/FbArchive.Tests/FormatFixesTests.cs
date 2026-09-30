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

  /// <summary>
  /// BKF edits used to put new files into the last folder of the backup and remove files by
  /// leaf name from every folder.
  /// </summary>
  [Test]
  public void Bkf_EditsKeepFoldersApart() {
    var bkf = Path.Combine(_root, "b.bkf");
    Directory.CreateDirectory(Path.Combine(_root, "d", "e"));
    System.IO.File.WriteAllText(Path.Combine(_root, "d", "a.txt"), "inner");
    Commands.Add(bkf, [new(File("a.txt", "outer"), "a.txt"), new(Path.Combine(_root, "d"), "d")], null, "Bkf");
    Commands.Add(bkf, [new(File("c.txt", "gamma"), "c.txt")], null);
    Assert.That(Names(bkf), Is.EquivalentTo(new[] { "a.txt", "c.txt", "d/a.txt" }));
    Commands.Remove(bkf, ["a.txt"], null);
    Assert.That(Names(bkf), Is.EquivalentTo(new[] { "c.txt", "d/a.txt" }));
    var dirs = ArchiveOperations.List(bkf, null).Where(e => e.IsDirectory).Select(e => Commands.Normalize(e.Name));
    Assert.That(dirs, Does.Contain("d/e"), "empty folder kept");
  }

  /// <summary>Adding to an MFS-1 image cut the other files' names down to their extension.</summary>
  [Test]
  public void Mfs1_AddingKeepsTheOtherNames() {
    var img = Path.Combine(_root, "m.mfsd");
    Commands.Add(img, [new(File("a.txt", "alpha"), "a.txt"), new(File("b.bin", "beta"), "b.bin")], null, "Mfs1");
    Commands.Add(img, [new(File("c.txt", "gamma"), "c.txt")], null);
    Assert.That(Names(img).Select(n => n.ToLowerInvariant()), Is.EquivalentTo(new[] { "a.txt", "b.bin", "c.txt" }));
    Commands.Remove(img, [Names(img).First(n => n.Equals("a.txt", StringComparison.OrdinalIgnoreCase))], null);
    Assert.That(Names(img).Select(n => n.ToLowerInvariant()), Is.EquivalentTo(new[] { "b.bin", "c.txt" }));
  }

  /// <summary>Formats whose files do not read back are not offered for new archives.</summary>
  [TestCase("StuffItX", ".sitx")]
  [TestCase("Mtree", ".mtree")]
  public void UnrecoverableWritersAreRefused(string format, string ext) {
    var target = Path.Combine(_root, "x" + ext);
    Assert.Throws<NotSupportedException>(() => Commands.Add(target, [new(File("a.txt", "alpha"), "a.txt")], null, format));
    Assert.That(System.IO.File.Exists(target), Is.False);
  }

  /// <summary>
  /// A ".bin" holding plain ISO sectors (as the library writes BIN images) is an ISO and can be
  /// edited like one; raw BIN/CUE images stay read-only, with the reason.
  /// </summary>
  [Test]
  public void Bin_WithIsoSectorsIsAnIso_RawBinCueIsReadOnly() {
    var bin = Path.Combine(_root, "d.bin");
    Commands.Add(bin, [new(File("a.txt", "alpha"), "a.txt")], null, "BinCue");
    Assert.That(FormatDetector.Detect(bin), Is.EqualTo(FormatDetector.Format.Iso));
    Assert.That(Commands.IsWritable(FormatDetector.Detect(bin), bin), Is.True);
    Assert.That(Commands.ReadOnlyReason(FormatDetector.Format.BinCue, bin), Does.Contain("sector"));
  }

  /// <summary>A generic suffix does not hide what a file is: a ZIP saved as ".bin" is a ZIP.</summary>
  [Test]
  public void GenericSuffix_ContentDecides() {
    var zip = Path.Combine(_root, "backup.zip");
    Commands.Add(zip, [new(File("a.txt", "alpha"), "a.txt")], null);
    var bin = Path.Combine(_root, "backup.bin");
    System.IO.File.Move(zip, bin);
    Assert.That(FormatDetector.Detect(bin), Is.EqualTo(FormatDetector.Format.Zip));
    Assert.That(Names(bin), Is.EqualTo(new[] { "a.txt" }));
    // A specific suffix still decides: a .jar is a ZIP underneath but stays a JAR.
    var jar = Path.Combine(_root, "x.jar");
    Commands.Add(jar, [new(File("b.txt", "beta"), "b.txt")], null, "Jar");
    Assert.That(FormatDetector.Detect(jar), Is.EqualTo(FormatDetector.Format.Jar));
  }}
