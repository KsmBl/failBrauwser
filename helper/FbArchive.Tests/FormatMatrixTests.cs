using System.Collections.Concurrent;
using System.Runtime.CompilerServices;
using Compression.Lib;
using Compression.Registry;

namespace FbArchive.Tests;

/// <summary>
/// Every format the library can write goes through what failBrauwser does with it: create
/// (with the format named explicitly, as the Compress dialog does), recognise, list, extract,
/// and for writable ones add and remove. The outcome of each format is compared with
/// <c>format-baseline.tsv</c>; a format that gets worse fails. <c>FB_REGEN_MATRIX=1</c>
/// rewrites the baseline after an improvement.
/// </summary>
[Parallelizable(ParallelScope.Children)]
public class FormatMatrixTests {
  private static readonly string[] Steps = ["create", "detect", "list", "extract", "add", "remove", "extract2", "ok"];
  private static readonly ConcurrentDictionary<string, string> Results = new();
  private static string BaselinePath([CallerFilePath] string here = "") => Path.Combine(Path.GetDirectoryName(here)!, "format-baseline.tsv");

  private static Dictionary<string, string> Baseline() =>
    File.Exists(BaselinePath())
      ? File.ReadAllLines(BaselinePath()).Where(l => l.Length > 0 && !l.StartsWith('#')).Select(l => l.Split('\t')).ToDictionary(p => p[0], p => string.Join('\t', p.Skip(1)))
      : [];

  public static IEnumerable<string> Creatable() {
    FormatRegistration.EnsureInitialized();
    return FormatRegistry.All
      .Where(d => d.Category is FormatCategory.Archive or FormatCategory.CompoundTar or FormatCategory.Stream or FormatCategory.Wrapper)
      .Where(d => d.Capabilities.HasFlag(FormatCapabilities.CanCreate))
      .Select(d => d.Id).OrderBy(x => x, StringComparer.Ordinal);
  }

  [OneTimeTearDown]
  public void WriteBaseline() {
    if (Environment.GetEnvironmentVariable("FB_REGEN_MATRIX") == null || Results.IsEmpty) return;
    var lines = new List<string> {
      "# Outcome of each writable format in FormatMatrixTests: the step it reached (ok = all),",
      "# and what it does differently (flat = folders not kept, case = names change case,",
      "# extra = entries of its own besides the files). Regenerate with FB_REGEN_MATRIX=1.",
    };
    lines.AddRange(Results.OrderBy(r => r.Key, StringComparer.Ordinal).Select(r => $"{r.Key}\t{r.Value}"));
    File.WriteAllLines(BaselinePath(), lines);
  }

  [Test]
  public void BaselineCoversEveryFormat() {
    if (Environment.GetEnvironmentVariable("FB_REGEN_MATRIX") != null) return;
    Assert.That(Baseline().Keys, Is.EquivalentTo(Creatable()), "format-baseline.tsv is out of date: FB_REGEN_MATRIX=1 dotnet test");
  }

  [TestCaseSource(nameof(Creatable))]
  public void RoundTrip(string id) {
    var got = Run(id);
    Results[id] = got;
    if (Environment.GetEnvironmentVariable("FB_REGEN_MATRIX") != null) return;
    if (!Baseline().TryGetValue(id, out var want)) Assert.Fail($"{id} is not in the baseline; got {got}");
    var (wantStep, wantFlags) = Parse(want);
    var (gotStep, gotFlags) = Parse(got);
    Assert.That(Array.IndexOf(Steps, gotStep), Is.GreaterThanOrEqualTo(Array.IndexOf(Steps, wantStep)), $"{id}: got {got}, baseline {want}");
    Assert.That(gotFlags.Except(wantFlags), Is.Empty, $"{id}: got {got}, baseline {want}");
  }

  private static (string Step, string[] Flags) Parse(string result) {
    var parts = result.Split('\t');
    var step = parts[0].StartsWith("fail ", StringComparison.Ordinal) ? parts[0][5..].Split(':')[0] : parts[0];
    return (step, parts.Length > 1 ? parts[1].Split(',', StringSplitOptions.RemoveEmptyEntries) : []);
  }

  private static string Run(string id) {
    var d = FormatRegistry.GetById(id)!;
    var root = Directory.CreateTempSubdirectory("fb-matrix-");
    var flags = new SortedSet<string>();
    var step = "create";
    try {
      var src = Path.Combine(root.FullName, "src");
      Directory.CreateDirectory(Path.Combine(src, "d"));
      File.WriteAllText(Path.Combine(src, "a.txt"), "hello world\n");
      var bin = new byte[3000];
      new Random(1).NextBytes(bin);
      File.WriteAllBytes(Path.Combine(src, "d", "b.bin"), bin);
      File.WriteAllText(Path.Combine(root.FullName, "c.txt"), "added later\n");
      var ext = d.CompoundExtensions.Count > 0 ? d.CompoundExtensions[0] : d.DefaultExtension;
      var arc = Path.Combine(root.FullName, "t" + ext);
      var single = d.Category is FormatCategory.Stream or FormatCategory.Wrapper || !d.Capabilities.HasFlag(FormatCapabilities.SupportsMultipleEntries);
      var dirs = !single && d.Capabilities.HasFlag(FormatCapabilities.SupportsDirectories);

      var task = Task.Run(() => {
        var items = single ? [new(Path.Combine(src, "a.txt"), "a.txt")]
          : dirs ? new List<AddItem> { new(Path.Combine(src, "a.txt"), "a.txt"), new(Path.Combine(src, "d"), "d") }
          : [new(Path.Combine(src, "a.txt"), "a.txt"), new(Path.Combine(src, "d", "b.bin"), "b.bin")];
        Commands.Add(arc, items, null, id);
        step = "detect";
        var detected = FormatDetector.Detect(arc).ToString();
        if (!string.Equals(detected, id, StringComparison.OrdinalIgnoreCase)) throw new Exception($"recognised as {detected}");
        step = "list";
        var want = single ? ["a.txt"] : dirs ? new[] { "a.txt", "d/b.bin" } : ["a.txt", "b.bin"];
        var names = Names(arc);
        var mapped = want.ToDictionary(w => w, w => Find(names, w, flags));
        if (mapped.Values.Any(v => v == null) && !single) throw new Exception("listed: " + string.Join(", ", names));
        if (names.Except(mapped.Values.OfType<string>()).Any() && !single) flags.Add("extra");
        step = "extract";
        var x = Path.Combine(root.FullName, "x");
        Commands.Extract(arc, x, null, null);
        if (single) {
          var files = Directory.GetFiles(x, "*", SearchOption.AllDirectories);
          if (!files.Any(f => File.ReadAllText(f) == "hello world\n")) throw new Exception("content differs: " + string.Join(", ", files.Select(Path.GetFileName)));
          return "ok";
        }
        if (File.ReadAllText(Path.Combine(x, mapped["a.txt"]!)) != "hello world\n") throw new Exception("a.txt differs");
        if (!File.ReadAllBytes(Path.Combine(x, mapped[want[1]]!)).SequenceEqual(bin)) throw new Exception("b.bin differs");
        if (!Commands.IsWritable(FormatDetector.Detect(arc), arc)) return "ok";
        step = "add";
        Commands.Add(arc, [new(Path.Combine(root.FullName, "c.txt"), "c.txt")], null);
        names = Names(arc);
        if (Find(names, "c.txt", flags) == null || Find(names, "a.txt", flags) == null) throw new Exception("after adding: " + string.Join(", ", names));
        step = "remove";
        Commands.Remove(arc, [Find(names, "a.txt", flags)!], null);
        names = Names(arc);
        if (Find(names, "a.txt", flags) != null || Find(names, "c.txt", flags) == null) throw new Exception("after removing: " + string.Join(", ", names));
        step = "extract2";
        var y = Path.Combine(root.FullName, "y");
        Commands.Extract(arc, y, null, null);
        if (File.ReadAllText(Path.Combine(y, Find(names, "c.txt", flags)!)) != "added later\n") throw new Exception("c.txt differs");
        return "ok";
      });
      var done = task.Wait(TimeSpan.FromSeconds(60)) ? task.Result : $"fail {step}: timeout";
      return flags.Count > 0 ? $"{done}\t{string.Join(',', flags)}" : done;
    }
    catch (AggregateException e) {
      var ie = e.InnerException!;
      var result = $"fail {step}: {ie.GetType().Name}: {ie.Message.Split('\n')[0].Trim().Replace(root.FullName, "<tmp>")}";
      return flags.Count > 0 ? $"{result}\t{string.Join(',', flags)}" : result;
    }
    finally {
      try { root.Delete(true); } catch (IOException) { } catch (UnauthorizedAccessException) { }
    }
  }

  private static List<string> Names(string arc) =>
    ArchiveOperations.List(arc, null).Where(e => !e.IsDirectory).Select(e => Commands.Normalize(e.Name)).ToList();

  /// <summary>The listed name for <paramref name="want"/>: exact, in another case, or without its folder.</summary>
  private static string? Find(List<string> names, string want, SortedSet<string> flags) {
    if (names.Contains(want)) return want;
    var ci = names.FirstOrDefault(n => string.Equals(n, want, StringComparison.OrdinalIgnoreCase));
    if (ci != null) { flags.Add("case"); return ci; }
    var leaf = want.Split('/')[^1];
    var flat = names.FirstOrDefault(n => string.Equals(n, leaf, StringComparison.OrdinalIgnoreCase));
    if (flat != null && leaf != want) { flags.Add("flat"); if (flat != leaf) flags.Add("case"); return flat; }
    return null;
  }
}
