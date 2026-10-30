using System.Text;
using Compression.Core.Checksums;

namespace FileFormat.Zip;

/// <summary>
/// Decodes the names stored in ZIP headers. The UTF-8 flag says the bytes are UTF-8; without
/// it the name is read as 7-Zip and Info-ZIP read it: from an Info-ZIP Unicode Path extra field
/// when it still matches the stored name, as UTF-8 when the bytes are valid UTF-8 (macOS,
/// Linux tools and phones write UTF-8 without setting the flag), else in the DOS code page
/// Windows Explorer writes names in (CP850, which agrees with CP437 on umlauts and ß).
/// </summary>
internal static class ZipNameEncoding {
  private const ushort UnicodePathExtraId = 0x7075;

  private static readonly Encoding StrictUtf8 = new UTF8Encoding(false, throwOnInvalidBytes: true);

  // CP850, bytes 0x80..0xFF.
  private const string Cp850High =
    "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜø£Ø×ƒáíóúñÑªº¿®¬½¼¡«»░▒▓│┤ÁÂÀ©╣║╗╝¢¥┐" +
    "└┴┬├─┼ãÃ╚╔╩╦╠═╬¤ðÐÊËÈıÍÎÏ┘┌█▄¦Ì▀ÓßÔÒõÕµþÞÚÛÙýÝ¯´­±‗¾¶§÷¸°¨·¹³²■ ";

  /// <summary>The name in <paramref name="raw"/>, given the entry's flags and extra field.</summary>
  public static string Decode(byte[] raw, ushort flags, byte[]? extra) {
    if ((flags & ZipConstants.FlagUtf8) != 0)
      return Encoding.UTF8.GetString(raw);
    if (UnicodePath(raw, extra) is { } unicode)
      return unicode;
    if (raw.All(b => b < 0x80))
      return Encoding.ASCII.GetString(raw);
    try {
      return StrictUtf8.GetString(raw);
    } catch (DecoderFallbackException) {
      return Cp850(raw);
    }
  }

  /// <summary>The bytes as CP850 text.</summary>
  public static string Cp850(byte[] raw) {
    var sb = new StringBuilder(raw.Length);
    foreach (var b in raw)
      sb.Append(b < 0x80 ? (char)b : Cp850High[b - 0x80]);
    return sb.ToString();
  }

  // The Info-ZIP Unicode Path extra field (0x7075): version 1, the CRC-32 of the stored
  // name, then the name in UTF-8. A tool that renamed the entry without updating the field
  // leaves a CRC that no longer matches; then the field is ignored.
  private static string? UnicodePath(byte[] raw, byte[]? extra) {
    if (extra == null) return null;
    for (var i = 0; i + 4 <= extra.Length;) {
      var id = (ushort)(extra[i] | extra[i + 1] << 8);
      var size = extra[i + 2] | extra[i + 3] << 8;
      var data = i + 4;
      if (data + size > extra.Length) return null;
      if (id == UnicodePathExtraId && size >= 5 && extra[data] == 1) {
        var crc = (uint)(extra[data + 1] | extra[data + 2] << 8 | extra[data + 3] << 16 | extra[data + 4] << 24);
        if (crc != Crc32.Compute(raw)) return null;
        try {
          return StrictUtf8.GetString(extra, data + 5, size - 5);
        } catch (DecoderFallbackException) {
          return null;
        }
      }
      i = data + size;
    }
    return null;
  }
}
