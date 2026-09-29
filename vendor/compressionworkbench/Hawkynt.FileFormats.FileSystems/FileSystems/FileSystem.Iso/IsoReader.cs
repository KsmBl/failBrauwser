#pragma warning disable CS1591
using Compression.Core.DiskImage;
using System.Text;

namespace FileSystem.Iso;

/// <summary>
/// Reads ISO 9660 (ECMA-119) disc images with optional Joliet and Rock Ridge support.
/// </summary>
public sealed class IsoReader : IDisposable {
  private const int SectorSize = 2048;
  private readonly ImageAccessor _data;
  private readonly List<IsoEntry> _entries = [];
  private readonly bool _preferJoliet;
  private readonly HashSet<long> _visitedDirectories = [];
  private bool _joliet;

  /// <summary>All entries found in the image.</summary>
  public IReadOnlyList<IsoEntry> Entries => _entries;

  /// <summary>
  /// Opens an ISO 9660 image from the given stream. When <paramref name="useJoliet"/>
  /// is <see langword="true"/> (the default) and the image carries a Joliet
  /// Supplementary Volume Descriptor, the long UCS-2 names from the Joliet
  /// directory tree are returned; otherwise the primary ECMA-119 tree is read.
  /// </summary>
  public IsoReader(Stream stream, bool leaveOpen = false, bool useJoliet = true) {
    ArgumentNullException.ThrowIfNull(stream);
    _preferJoliet = useJoliet;
    if (stream.CanSeek) stream.Position = 0;
    _data = new ImageAccessor(stream, leaveOpen);
    Parse();
  }

  private void Parse() {
    if (_data.Length < 17L * SectorSize)
      throw new InvalidDataException("ISO9660: image too small.");

    long pvdOffset = -1;
    long jolietOffset = -1;
    for (var sector = 16; sector < 256; ++sector) {
      var off = (long)sector * SectorSize;
      if (off + SectorSize > _data.Length) break;

      var type = _data.ReadByte(off);
      if (type == 0xFF) break;
      if (!IsCD001(off)) continue;

      if (type == 1 && pvdOffset < 0) {
        pvdOffset = off;
      } else if (type == 2 && jolietOffset < 0 && _preferJoliet) {
        var esc = _data.Read(off + 88, 3);
        if (esc is [0x25, 0x2F, 0x40 or 0x43 or 0x45])
          jolietOffset = off;
      }
    }

    if (pvdOffset < 0)
      throw new InvalidDataException("ISO9660: no Primary Volume Descriptor found.");

    var descOff = pvdOffset;
    // Rock Ridge carries full POSIX names (and modes); like Linux, prefer it over Joliet,
    // whose names stop at 64 characters.
    if (jolietOffset >= 0 && HasRockRidge(pvdOffset)) jolietOffset = -1;
    if (jolietOffset >= 0) {
      _joliet = true;
      descOff = jolietOffset;
    }

    var rootRec = descOff + 156;
    var rootExtent = _data.ReadUInt32(rootRec + 2);
    var rootLength = (long)_data.ReadUInt32(rootRec + 10);
    var rootExtendedAttributeBlocks = _data.ReadByte(rootRec + 1);
    var rootOffset = DataOffset(rootExtent, rootExtendedAttributeBlocks);
    ReadDirectory(rootOffset, rootLength, "");
  }

  /// <summary>
  /// Rock Ridge is present when the first record of the primary root directory carries a
  /// SUSP "SP" entry (IEEE P1281) at the start of its system use area.
  /// </summary>
  private bool HasRockRidge(long pvdOffset) {
    try {
      var rootRec = pvdOffset + 156;
      var root = DataOffset(_data.ReadUInt32(rootRec + 2), _data.ReadByte(rootRec + 1));
      var recLen = _data.ReadByte(root);
      var nameLen = _data.ReadByte(root + 32);
      var su = root + 33 + nameLen + ((nameLen & 1) == 0 ? 1 : 0);
      return recLen >= 40 && _data.ReadByte(su) == 'S' && _data.ReadByte(su + 1) == 'P'
        && _data.ReadByte(su + 4) == 0xBE && _data.ReadByte(su + 5) == 0xEF;
    } catch (Exception e) when (e is IOException or InvalidDataException or ArgumentOutOfRangeException) {
      return false;
    }
  }

  /// <summary>Rock Ridge fields of one directory record.</summary>
  private readonly record struct RockRidge(string? Name, int? Mode, DateTime? Modified);

  private RockRidge ReadRockRidge(long pos, int recLen, int nameLen) {
    if (_joliet) return default;
    var start = pos + 33 + nameLen + ((nameLen & 1) == 0 ? 1 : 0);
    var name = new List<byte>();
    var haveName = false;
    int? mode = null;
    DateTime? modified = null;
    // The system use area, then at most a few continuation areas (CE).
    var areas = new Queue<(long Start, long End)>();
    areas.Enqueue((start, pos + recLen));
    var hops = 0;
    while (areas.Count > 0 && hops++ < 8) {
      var (p, end) = areas.Dequeue();
      while (p + 4 <= end) {
        var s0 = _data.ReadByte(p);
        var s1 = _data.ReadByte(p + 1);
        var len = _data.ReadByte(p + 2);
        if (len < 4 || p + len > end) break;
        if (s0 == 'N' && s1 == 'M' && len >= 5) {
          var flags = _data.ReadByte(p + 4);
          if ((flags & 0x06) == 0) {
            name.AddRange(_data.Read(p + 5, len - 5));
            haveName = true;
          }
        } else if (s0 == 'P' && s1 == 'X' && len >= 12) {
          mode = (int)_data.ReadUInt32(p + 4);
        } else if (s0 == 'T' && s1 == 'F' && len >= 5) {
          modified = ReadRockRidgeModified(p, len);
        } else if (s0 == 'C' && s1 == 'E' && len >= 28) {
          var block = _data.ReadUInt32(p + 4);
          var offset = _data.ReadUInt32(p + 12);
          var length = _data.ReadUInt32(p + 20);
          var ceStart = (long)block * SectorSize + offset;
          if (ceStart + length <= _data.Length) areas.Enqueue((ceStart, ceStart + length));
        } else if (s0 == 'S' && s1 == 'T') {
          break;
        }
        p += len;
      }
    }
    return new RockRidge(haveName ? Encoding.UTF8.GetString(name.ToArray()) : null, mode, modified);
  }

  /// <summary>The modification time of a TF entry (7-byte or 17-byte form).</summary>
  private DateTime? ReadRockRidgeModified(long p, int len) {
    var flags = _data.ReadByte(p + 4);
    var longForm = (flags & 0x80) != 0;
    var size = longForm ? 17 : 7;
    var at = p + 5;
    if ((flags & 0x01) != 0) at += size; // creation comes first
    if ((flags & 0x02) == 0 || at + size > p + len) return null;
    try {
      if (longForm) {
        var s = Encoding.ASCII.GetString(_data.Read(at, 14));
        return DateTime.ParseExact(s, "yyyyMMddHHmmss", System.Globalization.CultureInfo.InvariantCulture, System.Globalization.DateTimeStyles.AssumeUniversal | System.Globalization.DateTimeStyles.AdjustToUniversal);
      }
      var t = new DateTime(_data.ReadByte(at) + 1900, _data.ReadByte(at + 1), _data.ReadByte(at + 2), _data.ReadByte(at + 3), _data.ReadByte(at + 4), _data.ReadByte(at + 5), DateTimeKind.Utc);
      var gmt = (sbyte)_data.ReadByte(at + 6);
      return t.AddMinutes(-15 * gmt);
    } catch (Exception e) when (e is ArgumentOutOfRangeException or FormatException) {
      return null;
    }
  }

  private bool IsCD001(long vdOffset) =>
    _data.Length > vdOffset + 5 &&
    _data.ReadByte(vdOffset + 1) == 'C' && _data.ReadByte(vdOffset + 2) == 'D' &&
    _data.ReadByte(vdOffset + 3) == '0' && _data.ReadByte(vdOffset + 4) == '0' && _data.ReadByte(vdOffset + 5) == '1';

  private void ReadDirectory(long offset, long length, string basePath) {
    if (offset < 0 || length < 0 || offset > _data.Length || length > _data.Length - offset)
      throw new InvalidDataException($"ISO9660: directory '{basePath}' extent lies outside the image.");
    if (!_visitedDirectories.Add(offset))
      throw new InvalidDataException($"ISO9660: directory '{basePath}' reuses an already visited extent.");

    var end = checked(offset + length);
    var pos = offset;
    PendingFile? pending = null;

    while (pos < end) {
      var recLen = _data.ReadByte(pos);
      if (recLen == 0) {
        pos = ((pos / SectorSize) + 1) * SectorSize;
        continue;
      }
      if (recLen < 34 || pos > end - recLen)
        throw new InvalidDataException($"ISO9660: malformed directory record in '{basePath}'.");

      var extendedAttributeBlocks = _data.ReadByte(pos + 1);
      var extentLba = _data.ReadUInt32(pos + 2);
      var dataLength = (long)_data.ReadUInt32(pos + 10);
      var flags = _data.ReadByte(pos + 25);
      var fileUnitSize = _data.ReadByte(pos + 26);
      var interleaveGapSize = _data.ReadByte(pos + 27);
      var nameLen = _data.ReadByte(pos + 32);
      var isDir = (flags & 0x02) != 0;
      var hasMoreExtents = (flags & 0x80) != 0;

      if (33 + nameLen > recLen)
        throw new InvalidDataException($"ISO9660: directory record in '{basePath}' has a truncated file identifier.");

      var isSpecial = nameLen == 1 && (_data.ReadByte(pos + 33) is 0 or 1);
      var rr = isSpecial ? default : ReadRockRidge(pos, recLen, nameLen);
      var name = rr.Name ?? DecodeName(pos, recLen, nameLen);
      if (!isSpecial && rr.Name is null) {
        var semi = name.IndexOf(';');
        if (semi >= 0) name = name[..semi];
        name = name.TrimEnd('.');
      }

      if (isSpecial) {
        if (pending is not null)
          throw new InvalidDataException($"ISO9660: multi-extent file '{pending.Name}' is interrupted by a special directory record.");
        pos += recLen;
        continue;
      }
      if (string.IsNullOrEmpty(name)) {
        if (pending is not null)
          throw new InvalidDataException($"ISO9660: multi-extent file '{pending.Name}' is interrupted by an empty identifier.");
        pos += recLen;
        continue;
      }

      var fullPath = string.IsNullOrEmpty(basePath) ? name : $"{basePath}/{name}";
      var physicalOffset = DataOffset(extentLba, extendedAttributeBlocks);
      var lastModified = rr.Modified ?? DecodeTimestamp(pos, end);

      if (isDir) {
        if (pending is not null)
          throw new InvalidDataException($"ISO9660: multi-extent file '{pending.Name}' is interrupted by directory '{fullPath}'.");
        if (hasMoreExtents)
          throw new InvalidDataException($"ISO9660: directory '{fullPath}' illegally advertises the Multi-Extent flag.");
        if (fileUnitSize != 0 || interleaveGapSize != 0)
          throw new NotSupportedException($"ISO9660: interleaved directory '{fullPath}' is not supported.");

        _entries.Add(new IsoEntry {
          Name = fullPath,
          IsDirectory = true,
          LastModified = lastModified,
          UnixMode = rr.Mode,
          DataOffset = physicalOffset,
        });
        ReadDirectory(physicalOffset, dataLength, fullPath);
        pos += recLen;
        continue;
      }

      string? limitation = null;
      if (fileUnitSize != 0 || interleaveGapSize != 0)
        limitation = $"ISO9660 interleaved file section uses FileUnitSize={fileUnitSize}, InterleaveGapSize={interleaveGapSize}.";

      if (pending is null) {
        var segments = new List<IsoDataSegment> { new(0, physicalOffset, dataLength) };
        if (hasMoreExtents) {
          pending = new PendingFile(fullPath, lastModified, segments, dataLength, limitation) { Mode = rr.Mode };
        } else {
          AddFile(fullPath, lastModified, segments, dataLength, limitation, rr.Mode);
        }
      } else {
        if (!string.Equals(pending.Name, fullPath, StringComparison.Ordinal))
          throw new InvalidDataException(
            $"ISO9660: multi-extent file '{pending.Name}' is followed by unrelated record '{fullPath}'.");
        var segmentOffset = pending.Length;
        pending.Segments.Add(new IsoDataSegment(segmentOffset, physicalOffset, dataLength));
        pending.Length = checked(pending.Length + dataLength);
        pending.Limitation ??= limitation;
        if (!hasMoreExtents) {
          AddFile(pending.Name, pending.LastModified, pending.Segments, pending.Length, pending.Limitation, pending.Mode);
          pending = null;
        }
      }

      pos += recLen;
    }

    if (pending is not null)
      throw new InvalidDataException($"ISO9660: multi-extent file '{pending.Name}' is missing its final directory record.");
  }

  private void AddFile(
      string name,
      DateTime? lastModified,
      IReadOnlyList<IsoDataSegment> segments,
      long length,
      string? limitation,
      int? mode = null) {
    _entries.Add(new IsoEntry {
      Name = name,
      Size = length,
      IsDirectory = false,
      LastModified = lastModified,
      UnixMode = mode,
      DataOffset = segments.Count == 1 ? segments[0].PhysicalOffset : 0,
      DataSegments = segments,
      MountLimitation = limitation,
    });
  }

  private string DecodeName(long pos, int recLen, int nameLen) {
    if (_joliet)
      return Encoding.BigEndianUnicode.GetString(_data.Read(pos + 33, nameLen));

    var suOffset = 33 + nameLen;
    if ((nameLen & 1) == 0) ++suOffset;
    return GetRockRidgeName(pos + suOffset, pos + recLen)
      ?? Encoding.ASCII.GetString(_data.Read(pos + 33, nameLen));
  }

  private DateTime? DecodeTimestamp(long pos, long end) {
    if (pos + 24 >= end) return null;
    try {
      var y = _data.ReadByte(pos + 18) + 1900;
      var m = _data.ReadByte(pos + 19);
      var d = _data.ReadByte(pos + 20);
      var h = _data.ReadByte(pos + 21);
      var mi = _data.ReadByte(pos + 22);
      var s = _data.ReadByte(pos + 23);
      return new DateTime(y, m, d, h, mi, s, DateTimeKind.Utc);
    } catch (ArgumentOutOfRangeException) {
      return null;
    }
  }

  private static long DataOffset(uint extentLba, byte extendedAttributeBlocks)
    => checked(((long)extentLba + extendedAttributeBlocks) * SectorSize);

  private string? GetRockRidgeName(long start, long end) {
    var pos = start;
    while (pos + 4 <= end) {
      var sig0 = _data.ReadByte(pos);
      var sig1 = _data.ReadByte(pos + 1);
      var len = _data.ReadByte(pos + 2);
      if (len < 4 || pos + len > end) break;
      if (sig0 == 'N' && sig1 == 'M' && len > 5)
        return Encoding.ASCII.GetString(_data.Read(pos + 5, len - 5));
      pos += len;
    }
    return null;
  }

  /// <summary>
  /// Copies an entry's logical bytes into <paramref name="destination"/>. Multi-extent
  /// file sections are concatenated in directory-record order as required by ECMA-119.
  /// </summary>
  public void ExtractTo(IsoEntry entry, Stream destination) {
    ArgumentNullException.ThrowIfNull(entry);
    ArgumentNullException.ThrowIfNull(destination);
    if (entry.IsDirectory) return;
    if (entry.MountLimitation is { } limitation)
      throw new NotSupportedException($"ISO9660: '{entry.Name}' cannot be decoded safely: {limitation}");

    long logical = 0;
    foreach (var segment in Segments(entry)) {
      if (segment.LogicalOffset != logical)
        throw new InvalidDataException($"ISO9660: '{entry.Name}' has a discontinuous file-section map.");
      if (segment.PhysicalOffset < 0 || segment.PhysicalOffset > _data.Length - segment.Length)
        throw new InvalidDataException($"ISO9660: '{entry.Name}' has a file section outside the image.");
      _data.CopyTo(segment.PhysicalOffset, destination, segment.Length);
      logical = checked(logical + segment.Length);
    }
    if (logical != entry.Size)
      throw new InvalidDataException($"ISO9660: '{entry.Name}' file sections produce {logical} of {entry.Size} bytes.");
  }

  /// <summary>Extracts the logical data for the given entry.</summary>
  public byte[] Extract(IsoEntry entry) {
    ArgumentNullException.ThrowIfNull(entry);
    if (entry.IsDirectory || entry.Size <= 0) return [];
    if (entry.Size > Array.MaxLength)
      throw new IOException($"ISO9660: '{entry.Name}' is {entry.Size:N0} bytes, past the array limit; use ExtractTo.");
    using var output = new MemoryStream(checked((int)entry.Size));
    ExtractTo(entry, output);
    return output.ToArray();
  }

  internal static IReadOnlyList<IsoDataSegment> Segments(IsoEntry entry)
    => entry.DataSegments.Count != 0
      ? entry.DataSegments
      : entry.Size <= 0
        ? []
        : [new(0, entry.DataOffset, entry.Size)];

  /// <inheritdoc/>
  public void Dispose() => _data.Dispose();

  private sealed class PendingFile(
    string name,
    DateTime? lastModified,
    List<IsoDataSegment> segments,
    long length,
    string? limitation
  ) {
    public string Name { get; } = name;
    public DateTime? LastModified { get; } = lastModified;
    public List<IsoDataSegment> Segments { get; } = segments;
    public long Length { get; set; } = length;
    public string? Limitation { get; set; } = limitation;
    public int? Mode { get; init; }
  }
}
