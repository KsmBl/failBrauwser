using FileSystem.Iso;

namespace FbArchive.Tests;

/// <summary>Rock Ridge and El Torito as failBrauwser added them to the vendored ISO code.</summary>
[TestFixture]
public class IsoTests {

  private static byte[] Build(Action<IsoWriter> fill) {
    var w = new IsoWriter { EnableRockRidge = true, EnableJoliet = true, VolumeIdentifier = "TEST" };
    fill(w);
    return w.Build();
  }

  [Test]
  public void RockRidge_RoundTrips_NamesModesTimesAndEmptyDirectories() {
    var when = new DateTime(2024, 5, 6, 7, 8, 9, DateTimeKind.Utc);
    var name = "Ünïcode and spaces — much longer than the sixty-four characters of Joliet.txt";
    var iso = Build(w => {
      w.AddDirectory("empty folder", 0x1C0, when);
      w.AddFile("dir/" + name, [1, 2, 3]);
      w.SetMetadata("dir/" + name, 0x1ED, when);
      w.AddFile("dir/Same.txt", [4]);
      w.AddFile("dir/same.txt", [5]);
    });
    using var r = new IsoReader(new MemoryStream(iso));
    var byName = r.Entries.ToDictionary(e => e.Name);
    Assert.That(byName.ContainsKey("empty folder"), Is.True);
    Assert.That(byName["empty folder"].UnixMode & 0xFFF, Is.EqualTo(0x1C0));
    var f = byName["dir/" + name];
    Assert.That(f.UnixMode & 0xFFF, Is.EqualTo(0x1ED));
    Assert.That(f.LastModified, Is.EqualTo(when));
    Assert.That(r.Extract(f), Is.EqualTo(new byte[] { 1, 2, 3 }));
    // Names that differ only in case both survive (unique ISO identifiers underneath).
    Assert.That(r.Extract(byName["dir/Same.txt"]), Is.EqualTo(new byte[] { 4 }));
    Assert.That(r.Extract(byName["dir/same.txt"]), Is.EqualTo(new byte[] { 5 }));
  }

  [Test]
  public void NewImage_KeepsFoldersAndLongNames() {
    var src = Directory.CreateTempSubdirectory("fb-iso-new-");
    try {
      var name = "A long name with Case, longer than the old 8.3 limit.txt";
      Directory.CreateDirectory(Path.Combine(src.FullName, "pack", "sub", "empty"));
      File.WriteAllText(Path.Combine(src.FullName, "pack", "sub", name), "deep");
      File.WriteAllText(Path.Combine(src.FullName, "top.txt"), "top");
      var iso = Path.Combine(src.FullName, "new.iso");
      Commands.Add(iso, [new(Path.Combine(src.FullName, "pack"), "pack"), new(Path.Combine(src.FullName, "top.txt"), "top.txt")], null, "Iso");
      using var r = new IsoReader(File.OpenRead(iso));
      var byName = r.Entries.ToDictionary(e => e.Name);
      Assert.That(byName.Keys, Is.SupersetOf(new[] { "top.txt", "pack", "pack/sub", "pack/sub/empty", "pack/sub/" + name }));
      Assert.That(r.Extract(byName["pack/sub/" + name]), Is.EqualTo("deep"u8.ToArray()));
    }
    finally {
      src.Delete(true);
    }
  }

  [Test]
  public void ElTorito_EntriesPointAtTheirFiles_AndBootInfoTableIsPatched() {
    var bios = new byte[4096];
    new Random(1).NextBytes(bios);
    var efi = new byte[8192];
    new Random(2).NextBytes(efi);
    var iso = Build(w => {
      w.AddFile("boot/isolinux.bin", bios);
      w.AddFile("boot/efi.img", efi);
      w.AddFile("boot/boot.cat", new byte[2048]);
      w.BootCatalogPath = "boot/boot.cat";
      w.BootEntries.Add(new IsoWriter.BootEntry { Platform = 0, SectorCount = 4, ImagePath = "boot/isolinux.bin", PatchBootInfoTable = true });
      w.BootEntries.Add(new IsoWriter.BootEntry { Platform = 0xEF, SectorCount = 16, ImagePath = "boot/efi.img" });
    });
    using var s = new MemoryStream(iso);
    var boot = IsoBoot.ReadElTorito(s);
    Assert.That(boot, Is.Not.Null);
    var (entries, catalog) = boot!.Value;
    Assert.That(entries.Select(e => e.Platform), Is.EqualTo(new byte[] { 0, 0xEF }));
    s.Position = 0;
    using var r = new IsoReader(s, leaveOpen: true);
    var byName = r.Entries.ToDictionary(e => e.Name);
    Assert.That(byName["boot/isolinux.bin"].FirstSector, Is.EqualTo(entries[0].LoadRba));
    Assert.That(byName["boot/efi.img"].FirstSector, Is.EqualTo(entries[1].LoadRba));
    Assert.That(byName["boot/boot.cat"].FirstSector, Is.EqualTo(catalog));
    var patched = r.Extract(byName["boot/isolinux.bin"]);
    Assert.That(IsoBoot.HasBootInfoTable(patched, entries[0].LoadRba), Is.True);
    Assert.That(patched[64..], Is.EqualTo(bios[64..]));
    Assert.That(r.Extract(byName["boot/efi.img"]), Is.EqualTo(efi));
    Assert.That(IsoBoot.IsHybrid(s), Is.False);
    Assert.That(IsoBoot.ReadVolumeId(s), Is.EqualTo("TEST"));
  }

  [Test]
  public void WithoutRockRidge_OutputIsUnchanged() {
    // The additions are opt-in: a plain image carries no SUSP entries.
    var w = new IsoWriter();
    w.AddFile("A.TXT", [1]);
    var iso = w.Build();
    Assert.That(System.Text.Encoding.ASCII.GetString(iso).Contains("RRIP_1991A"), Is.False);
    using var r = new IsoReader(new MemoryStream(iso));
    Assert.That(r.Entries.Single().Name, Is.EqualTo("A.TXT"));
  }
}
