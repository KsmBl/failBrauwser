using System.Text;

namespace FbArchive.Tests;

/// <summary>
/// Names in zips that do not set the UTF-8 flag: from Windows Explorer (DOS code page),
/// from macOS, Linux tools and phones (UTF-8 without the flag), and with an Info-ZIP
/// Unicode Path field.
/// </summary>
[TestFixture]
public class ZipNameTests {

  private string _root = "";

  [SetUp]
  public void MakeRoot() => _root = Directory.CreateTempSubdirectory("fb-zipname-").FullName;

  [TearDown]
  public void DropRoot() => Directory.Delete(_root, true);

  /// <summary>A stored zip of one-entry-per-name, written byte by byte: no UTF-8 flag.</summary>
  private string Zip(string file, params (byte[] Name, byte[]? Extra, string Content)[] entries) {
    using var ms = new MemoryStream();
    using var w = new BinaryWriter(ms);
    var central = new MemoryStream();
    using var cw = new BinaryWriter(central);
    foreach (var (name, extra, content) in entries) {
      var data = Encoding.ASCII.GetBytes(content);
      var crc = Crc(data);
      var offset = (uint)ms.Position;
      var ex = extra ?? [];
      w.Write(0x04034b50u); w.Write((ushort)20); w.Write((ushort)0); w.Write((ushort)0);
      w.Write((ushort)0); w.Write((ushort)0x21); w.Write(crc); w.Write((uint)data.Length); w.Write((uint)data.Length);
      w.Write((ushort)name.Length); w.Write((ushort)ex.Length); w.Write(name); w.Write(ex); w.Write(data);
      cw.Write(0x02014b50u); cw.Write((ushort)20); cw.Write((ushort)20); cw.Write((ushort)0); cw.Write((ushort)0);
      cw.Write((ushort)0); cw.Write((ushort)0x21); cw.Write(crc); cw.Write((uint)data.Length); cw.Write((uint)data.Length);
      cw.Write((ushort)name.Length); cw.Write((ushort)ex.Length); cw.Write((ushort)0); cw.Write((ushort)0);
      cw.Write((ushort)0); cw.Write(0x20u); cw.Write(offset); cw.Write(name); cw.Write(ex);
    }
    var cdOffset = (uint)ms.Position;
    w.Write(central.ToArray());
    w.Write(0x06054b50u); w.Write((ushort)0); w.Write((ushort)0); w.Write((ushort)entries.Length); w.Write((ushort)entries.Length);
    w.Write((uint)central.Length); w.Write(cdOffset); w.Write((ushort)0);
    var path = Path.Combine(_root, file);
    System.IO.File.WriteAllBytes(path, ms.ToArray());
    return path;
  }

  private static uint Crc(byte[] data) {
    var crc = 0xFFFFFFFFu;
    foreach (var b in data) {
      crc ^= b;
      for (var k = 0; k < 8; k++) crc = (crc & 1) != 0 ? (crc >> 1) ^ 0xEDB88320u : crc >> 1;
    }
    return ~crc;
  }

  /// <summary>An Info-ZIP Unicode Path field (0x7075) for a stored name.</summary>
  private static byte[] UnicodePath(byte[] storedName, string name) {
    var utf8 = Encoding.UTF8.GetBytes(name);
    var field = new List<byte> { 0x75, 0x70 };
    field.AddRange(BitConverter.GetBytes((ushort)(5 + utf8.Length)));
    field.Add(1);
    field.AddRange(BitConverter.GetBytes(Crc(storedName)));
    field.AddRange(utf8);
    return field.ToArray();
  }

  private List<string> Names(string zip) => Compression.Lib.ArchiveOperations.List(zip, null).Select(e => Commands.Normalize(e.Name)).OrderBy(n => n, StringComparer.Ordinal).ToList();

  [Test]
  public void WindowsExplorerNamesAreInTheDosCodePage() {
    // "Ürlaub Straße.jpg" and "Fotos/Grüße.png" as Explorer writes them (CP437/CP850).
    var zip = Zip("explorer.zip",
      ([0x9A, .. "rlaub Stra"u8, 0xE1, .. "e.jpg"u8], null, "photo"),
      ([.. "Fotos/Gr"u8, 0x81, 0xE1, .. "e.png"u8], null, "greeting"));
    Assert.That(Names(zip), Is.EquivalentTo(new[] { "Fotos/Grüße.png", "Ürlaub Straße.jpg" }));
    var x = Path.Combine(_root, "x");
    Commands.Extract(zip, x, null, null);
    Assert.That(System.IO.File.ReadAllText(Path.Combine(x, "Ürlaub Straße.jpg")), Is.EqualTo("photo"));
    Assert.That(System.IO.File.ReadAllText(Path.Combine(x, "Fotos", "Grüße.png")), Is.EqualTo("greeting"));
  }

  [Test]
  public void Utf8WithoutTheFlagStaysUtf8() {
    var zip = Zip("mac.zip", (Encoding.UTF8.GetBytes("Ürlaub/Café.jpg"), null, "x"), ("plain.txt"u8.ToArray(), null, "y"));
    Assert.That(Names(zip), Does.Contain("Ürlaub/Café.jpg").And.Contain("plain.txt"));
  }

  [Test]
  public void UnicodePathFieldWinsWhileItMatches() {
    byte[] stored = [.. "Gr"u8, 0x81, .. "n.txt"u8]; // CP850 "Grün.txt"
    var zip = Zip("unicode.zip",
      (stored, UnicodePath(stored, "Grün – 绿.txt"), "a"),
      // A field left over from before a rename no longer matches its name: ignored.
      ([.. "Bl"u8, 0x81, .. "te.txt"u8], UnicodePath("old.txt"u8.ToArray(), "Stale.txt"), "b"));
    Assert.That(Names(zip), Is.EquivalentTo(new[] { "Blüte.txt", "Grün – 绿.txt" }));
  }

  [Test]
  public void RenamedNamesAreWrittenBackAsUtf8() {
    // Adding to such a zip keeps the old names readable for every tool: written with the flag.
    var zip = Zip("edit.zip", ([0x9A, .. "ber.txt"u8], null, "old"));
    var add = Path.Combine(_root, "new.txt");
    System.IO.File.WriteAllText(add, "new");
    Commands.Add(zip, [new(add, "new.txt")], null);
    Assert.That(Names(zip), Is.EquivalentTo(new[] { "new.txt", "Über.txt" }));
  }
}
