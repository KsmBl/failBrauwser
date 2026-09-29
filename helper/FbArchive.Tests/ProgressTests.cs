using Compression.Lib;
using Compression.Registry;

namespace FbArchive.Tests;

/// <summary>The read counter failBrauwser added to the vendored library.</summary>
[TestFixture]
public class ProgressTests {

  [Test]
  public void ReadObserver_SeesEveryInputByte() {
    var dir = Directory.CreateTempSubdirectory("fb_progress_").FullName;
    try {
      var a = Path.Combine(dir, "a.bin");
      var b = Path.Combine(dir, "b.bin");
      File.WriteAllBytes(a, new byte[12345]);
      File.WriteAllBytes(b, new byte[678]);
      long seen = 0;
      ArchiveInputInfo.ReadObserver = n => Interlocked.Add(ref seen, n);
      try {
        ArchiveOperations.Create(Path.Combine(dir, "t.tar"), [new ArchiveInput(a, "a.bin"), new ArchiveInput(b, "b.bin")], new CompressionOptions());
      } finally {
        ArchiveInputInfo.ReadObserver = null;
      }
      Assert.That(seen, Is.EqualTo(12345 + 678));
    } finally {
      Directory.Delete(dir, true);
    }
  }
}
