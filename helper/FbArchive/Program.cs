using System.Text;
using System.Text.Json;
using Compression.Lib;

namespace FbArchive;

/// <summary>
/// Line based JSON protocol. Every request is one JSON object on one line of stdin and gets
/// exactly one JSON object on one line of stdout. Requests are handled strictly in order.
/// </summary>
public static class Program {
  public const int ProtocolVersion = 1;

  public static int Main(string[] args) {
    if (args.Length > 0 && args[0] == "--version") {
      Console.WriteLine($"fb-archive protocol {ProtocolVersion}");
      return 0;
    }

    var stdin = new StreamReader(Console.OpenStandardInput(), new UTF8Encoding(false));
    var stdout = Console.OpenStandardOutput();
    string? line;
    while ((line = stdin.ReadLine()) != null) {
      if (line.Length == 0) continue;
      var response = Handle(line, out var quit);
      stdout.Write(response);
      stdout.WriteByte((byte)'\n');
      stdout.Flush();
      if (quit) break;
    }
    return 0;
  }

  /// <summary>Handles one request line and returns the UTF-8 encoded response line.</summary>
  public static byte[] Handle(string line, out bool quit) {
    quit = false;
    long id = 0;
    using var buffer = new MemoryStream();
    using (var w = new Utf8JsonWriter(buffer)) {
      try {
        using var doc = JsonDocument.Parse(line);
        var req = doc.RootElement;
        id = req.TryGetProperty("id", out var idEl) && idEl.ValueKind == JsonValueKind.Number ? idEl.GetInt64() : 0;
        var cmd = req.GetProperty("cmd").GetString() ?? "";
        w.WriteStartObject();
        w.WriteNumber("id", id);
        w.WriteBoolean("ok", true);
        switch (cmd) {
          case "hello": w.WriteNumber("version", ProtocolVersion); break;
          case "quit": quit = true; break;
          case "formats": Commands.Formats(w); break;
          case "probe": Commands.Probe(w, Str(req, "path")); break;
          case "list": Commands.List(w, Str(req, "archive"), OptStr(req, "password")); break;
          case "extract":
            Commands.Extract(Str(req, "archive"), Str(req, "dest"), OptStrArray(req, "entries"), OptStr(req, "password"));
            break;
          case "add": Commands.Add(Str(req, "archive"), Items(req), OptStr(req, "password")); break;
          case "remove": Commands.Remove(Str(req, "archive"), OptStrArray(req, "names") ?? [], OptStr(req, "password")); break;
          case "rename": Commands.Rename(Str(req, "archive"), Str(req, "from"), Str(req, "to"), OptStr(req, "password")); break;
          case "mkdir": Commands.Mkdir(Str(req, "archive"), Str(req, "name"), OptStr(req, "password")); break;
          default: throw new ProtocolException($"unknown command '{cmd}'");
        }
        w.WriteEndObject();
      }
      catch (Exception ex) {
        w.Reset();
        buffer.SetLength(0);
        w.WriteStartObject();
        w.WriteNumber("id", id);
        w.WriteBoolean("ok", false);
        w.WriteString("code", Classify(ex));
        w.WriteString("error", Flatten(ex));
        w.WriteEndObject();
      }
    }
    return buffer.ToArray();
  }

  private static string Classify(Exception ex) {
    for (var e = ex; e != null; e = e.InnerException) {
      var msg = e.Message;
      if (msg.Contains("password", StringComparison.OrdinalIgnoreCase) || msg.Contains("encrypt", StringComparison.OrdinalIgnoreCase))
        return "password";
      if (e is NotSupportedException) return "unsupported";
      if (e is FileNotFoundException or DirectoryNotFoundException) return "notfound";
      if (e is UnauthorizedAccessException) return "permission";
      if (e is ProtocolException) return "protocol";
    }
    return "error";
  }

  private static string Flatten(Exception ex) {
    var sb = new StringBuilder(ex.Message);
    for (var e = ex.InnerException; e != null; e = e.InnerException) sb.Append(": ").Append(e.Message);
    return sb.ToString();
  }

  private static string Str(JsonElement req, string name)
    => req.TryGetProperty(name, out var el) && el.ValueKind == JsonValueKind.String
      ? el.GetString()!
      : throw new ProtocolException($"missing string field '{name}'");

  private static string? OptStr(JsonElement req, string name)
    => req.TryGetProperty(name, out var el) && el.ValueKind == JsonValueKind.String ? el.GetString() : null;

  private static string[]? OptStrArray(JsonElement req, string name) {
    if (!req.TryGetProperty(name, out var el) || el.ValueKind != JsonValueKind.Array) return null;
    return el.EnumerateArray().Select(e => e.GetString() ?? "").ToArray();
  }

  private static List<AddItem> Items(JsonElement req) {
    if (!req.TryGetProperty("items", out var el) || el.ValueKind != JsonValueKind.Array)
      throw new ProtocolException("missing array field 'items'");
    return el.EnumerateArray().Select(i => new AddItem(OptStr(i, "src") ?? "", Str(i, "name"))).ToList();
  }
}

public sealed class ProtocolException(string message) : Exception(message);

/// <summary>A file or directory on disk (<see cref="Src"/>) to be stored under <see cref="Name"/>.
/// An empty <see cref="Src"/> with a name ending in '/' creates an empty directory.</summary>
public readonly record struct AddItem(string Src, string Name);
