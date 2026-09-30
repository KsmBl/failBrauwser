# CompressionWorkbench (vendored)

The archive support of failBrauwser is the work of **[@Hawkynt](https://github.com/Hawkynt)**:
[CompressionWorkbench](https://github.com/Hawkynt/CompressionWorkbench), a library that reads
and writes hundreds of archive, compression and file system formats. failBrauwser only adds a
small helper process (`helper/FbArchive`) that speaks to it.

This folder holds the parts of CompressionWorkbench that `Compression.Lib` needs, so
failBrauwser builds on its own:

| Project | What it is |
|---|---|
| `Compression.Lib` | format detection and archive operations (list, extract, add, remove, create) |
| `Compression.Core` | compression primitives and shared containers (ZIP, …) |
| `Compression.Registry`, `Compression.Registry.Generator` | format contracts and their source-generated registration |
| `Hawkynt.FileFormats.Archives`, `.FileSystems`, `.Audio` | the formats themselves |
| `Hawkynt.Algorithms.Checksums`, `.Hashing` | checksums and hashes |

Upstream: https://github.com/Hawkynt/CompressionWorkbench, based on commit `8a4df931`
("README rewritten for the Linux GTK desktop app").

License: LGPL-3.0-or-later (`LICENSE` here); third-party notices in
`THIRD-PARTY-NOTICES.md`. The NuGet package `Hawkynt.FileFormats.Images` (also by Hawkynt)
is fetched at build time.

## Changes made for failBrauwser

- `39965dfa` (test) rebuild edit semantics: exact names, folders, case, times
- `950b150e` (fix) tar keeps times and executable bits, lists times as UTC
- `478d366d` (fix) zip keeps entry times and adds folder entries to existing archives
- `0f702049` (feat) inputs carry source timestamp and mode; helper to restore entry times on extract
- `9350e56f` (fix) rebuild-based add and remove no longer match entries by leaf name or drop empty folders
- `7ad59278` (fix) shared rebuild staging: exact entry names, folders removed with their contents
- (feat) `ArchiveInputInfo.ReadObserver` reports bytes read, for progress bars
- (fix) ISO reader prefers Rock Ridge names and reads PX, TF, NM continuations and CE areas
- (feat) ISO writer: Rock Ridge, folders, file metadata, unique ISO 9660 identifiers
- (feat) `IsoBoot`: read El Torito entries, detect hybrid images; the writer writes boot
  catalogs with BIOS and UEFI entries and patches the isolinux boot info table
- (fix) format ids are matched regardless of case (xDisk, xMash were never recognised)
- (fix) new ISO images keep folders, with Rock Ridge; CDI, MDF, NRG and BIN images reuse it
- (fix) ext2/3/4: new images keep folders, empty ones too; rebuilds keep and add folders
- (fix) NTFS: new images keep folders; folders can be added and removed
- (fix) XAR: table-of-contents checksum, folders and modes; edits keep the tree
- (fix) FAT: empty folders; VHD, VHDX, VMDK, VDI, QCOW2 keep long names and folders
- (bug) BBC DFS removed files of the same name in another directory; extraction names
- (bug) BKF edits went to the wrong folder and removed same-named files everywhere
- (bug) the shared rebuild path dropped entries by leaf name; it keeps folders now
- (bug) MFS-1 adds cut the other names down to their extension
- (fix) behind a generic suffix (.bin, .img, .dat, …) a clear signature decides the format

Later changes are in failBrauwser's git history (`git log -- vendor/compressionworkbench`);
their tests are in `helper/FbArchive.Tests`. They are candidates for upstream pull requests.
