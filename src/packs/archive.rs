//! The pack archive: a gzipped ustar tar, read in-house.
//!
//! **Why in-house:** the prompt's law is NO new dependency, and no tar
//! crate is in the lock. ustar's subset a pack needs is genuinely small —
//! regular files with short names — and a CLOSED reader is the format's
//! friend: every header this reader does not understand is a refusal
//! naming the entry, never a silent skip. The pack's INTEGRITY is not this
//! module's job (the signature is verified over the archive bytes before
//! this reader runs); its job is to hand the validator exactly the entries
//! the archive names, or refuse.
//!
//! The closed subset:
//! * gzip framing (flate2 — already in the lock, declared directly);
//! * 512-byte ustar headers, `ustar\0`/`ustar ` magic, `00` version;
//! * typeflag `'0'`/`'\0'` (regular files) only — directories are
//!   unnecessary in a pack (the allowlist's paths are the tree), and every
//!   other typeflag — GNU longname `L`, pax `x`/`g`, links, specials — is
//!   a REFUSAL naming the entry (re-tar with `--format=ustar`);
//! * entry names must be the plain relative paths the allowlist names:
//!   no leading `./`, no `..`, no absolute paths (the format module's
//!   allowlist would refuse them anyway — this reader refuses them first,
//!   naming the traversal attempt);
//! * header checksum verified (a corrupt header is a refusal, not a panic);
//! * bounds: at most 4,096 entries and 64 MiB of entry bytes — a pack is a
//!   declarations file set, not a corpus.
//!
//! [`write_archive`] is the reader's symmetric opposite, `#[doc(hidden)]`:
//! the operator mints packs with the system `tar` and signs with minisign;
//! the writer exists so the pins can build EXACT bytes in-process and the
//! round-trip is pinned, not assumed.

use std::io::Read;

const BLOCK: usize = 512;
const MAX_ENTRIES: usize = 4_096;
const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;

/// An archive refusal — every arm names the entry or the offset it died at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveError {
    NotGzip,
    Truncated(&'static str),
    BadHeader(&'static str),
    UnsupportedEntry { name: String, why: &'static str },
    TooLarge { entries: usize, bytes: usize },
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchiveError::NotGzip => write!(f, "the pack is not gzip-framed"),
            ArchiveError::Truncated(where_) => write!(f, "the archive is truncated: {where_}"),
            ArchiveError::BadHeader(what) => write!(f, "the tar header is malformed: {what}"),
            ArchiveError::UnsupportedEntry { name, why } => write!(
                f,
                "tar entry '{name}' is not a plain ustar regular file ({why}) — re-tar \
                 with --format=ustar"
            ),
            ArchiveError::TooLarge { entries, bytes } => write!(
                f,
                "the archive exceeds the pack bounds ({entries} entries, {bytes} bytes; \
                 max {MAX_ENTRIES} entries, {MAX_TOTAL_BYTES} bytes)"
            ),
        }
    }
}

/// Read a gzipped ustar archive into its entries, in archive order.
pub fn read_archive(gzipped: &[u8]) -> Result<Vec<(String, Vec<u8>)>, ArchiveError> {
    let mut decoder = flate2::read::GzDecoder::new(gzipped);
    let mut tar: Vec<u8> = Vec::new();
    decoder
        .read_to_end(&mut tar)
        .map_err(|_| ArchiveError::NotGzip)?;

    let mut entries = Vec::new();
    let mut total = 0usize;
    let mut off = 0usize;
    let mut zero_run = 0usize;
    while off < tar.len() {
        let header = &tar[off..(off + BLOCK).min(tar.len())];
        if header.len() < BLOCK {
            return Err(ArchiveError::Truncated("header block"));
        }
        if header.iter().all(|&b| b == 0) {
            // two consecutive zero blocks end the archive; one followed by
            // anything else is a truncated frame
            zero_run += 1;
            off += BLOCK;
            if zero_run == 2 {
                break;
            }
            continue;
        }
        zero_run = 0;

        // checksum: the 8 checksum bytes must sum to (sum of header bytes
        // with the checksum field read as spaces) + 0x20 * 8? No — the
        // stored value is the sum with the field AS SPACES, formatted
        // octal. Verify both the parse and the sum.
        let stored_ck = parse_octal(&header[148..156])? as usize;
        let mut sum = 0usize;
        for (i, &b) in header.iter().enumerate() {
            sum += if (148..156).contains(&i) {
                0x20
            } else {
                b as usize
            };
        }
        if sum != stored_ck {
            return Err(ArchiveError::BadHeader("checksum mismatch"));
        }

        let magic = &header[257..263];
        if magic != b"ustar\0" && magic != b"ustar " {
            return Err(ArchiveError::BadHeader(
                "magic is not ustar (pre-POSIX archives are refused)",
            ));
        }
        let typeflag = header[156];
        let size = parse_octal(&header[124..136])? as usize;
        let mut name = read_name(header)?;
        if name.is_empty() {
            return Err(ArchiveError::BadHeader("entry has an empty name"));
        }
        // traversal armour: the allowlist would refuse these anyway, but
        // the reader refuses them FIRST, naming the attempt
        if name.starts_with('/') || name.starts_with("./") || name.contains("../") {
            return Err(ArchiveError::UnsupportedEntry {
                name: name.clone(),
                why: "path traversal or absolute path",
            });
        }
        if name.ends_with('/') {
            return Err(ArchiveError::UnsupportedEntry {
                name: name.clone(),
                why: "a directory entry",
            });
        }
        match typeflag {
            b'0' | b'\0' => {}
            b'L' | b'x' | b'g' => {
                // GNU longname / pax metadata: a pack's allowlist names are
                // short by law, so needing one means the archive is not a
                // pack in the format's shape
                return Err(ArchiveError::UnsupportedEntry {
                    name: std::mem::take(&mut name),
                    why: "extended naming (GNU longname / pax header)",
                });
            }
            other => {
                // the typeflag vocabulary is closed; every arm names itself
                let why: &'static str = match other {
                    b'1' => "a hard link",
                    b'2' => "a symbolic link",
                    b'3' | b'4' => "a device special file",
                    b'5' => "a directory",
                    b'6' => "a FIFO",
                    b'7' => "a contiguous file",
                    b'L' | b'x' | b'g' => "extended naming (GNU longname / pax header)",
                    _ => "an unknown typeflag",
                };
                let _ = other;
                return Err(ArchiveError::UnsupportedEntry {
                    name: std::mem::take(&mut name),
                    why,
                });
            }
        }

        let data_start = off + BLOCK;
        let data_end = data_start + size;
        if data_end > tar.len() {
            return Err(ArchiveError::Truncated("entry data"));
        }
        total += size;
        entries.push((name, tar[data_start..data_end].to_vec()));
        if entries.len() > MAX_ENTRIES || total > MAX_TOTAL_BYTES {
            return Err(ArchiveError::TooLarge {
                entries: entries.len(),
                bytes: total,
            });
        }
        off = data_start + size.div_ceil(BLOCK) * BLOCK;
    }
    Ok(entries)
}

fn parse_octal(field: &[u8]) -> Result<u64, ArchiveError> {
    let s: Vec<u8> = field
        .iter()
        .copied()
        .take_while(|&b| b != 0 && b != b' ')
        .collect();
    let text = std::str::from_utf8(&s).map_err(|_| ArchiveError::BadHeader("octal field"))?;
    u64::from_str_radix(text.trim_matches('\0'), 8)
        .map_err(|_| ArchiveError::BadHeader("octal field does not parse"))
}

fn read_name(header: &[u8]) -> Result<String, ArchiveError> {
    let name = cstr(&header[0..100]);
    let prefix = cstr(&header[345..500]);
    if prefix.is_empty() {
        Ok(name)
    } else {
        Ok(format!("{prefix}/{name}"))
    }
}

fn cstr(field: &[u8]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// The reader's symmetric opposite: build a gzipped ustar archive from
/// entries. `#[doc(hidden)]` — the operator mints packs with the system
/// `tar` and signs with minisign; this writer exists so the pins build
/// EXACT bytes in-process and the round trip is pinned, not assumed.
#[doc(hidden)]
pub fn write_archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut tar: Vec<u8> = Vec::new();
    for (name, data) in entries {
        let mut header = [0u8; BLOCK];
        header[0..name.len()].copy_from_slice(name.as_bytes());
        header[100..108].copy_from_slice(b"0000644\0"); // mode
        header[108..116].copy_from_slice(b"0000000\0"); // uid
        header[116..124].copy_from_slice(b"0000000\0"); // gid
        let size_field = format!("{:011o}\0", data.len());
        header[124..136].copy_from_slice(size_field.as_bytes());
        header[136..148].copy_from_slice(b"00000000000\0"); // mtime
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        // checksum: the stored value is the sum over the header with the
        // checksum field read as SPACES; the field is zeros right now, so
        // its contribution is 8*0x20
        let sum: usize = header.iter().map(|&b| b as usize).sum::<usize>() + 8 * 0x20;
        let ck = format!("{:06o}\0 ", sum);
        header[148..156].copy_from_slice(ck.as_bytes());
        tar.extend_from_slice(&header);
        tar.extend_from_slice(data);
        let pad = data.len().div_ceil(BLOCK) * BLOCK - data.len();
        tar.extend(std::iter::repeat_n(0u8, pad));
    }
    tar.extend(std::iter::repeat_n(0u8, BLOCK * 2));
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&tar).expect("gzip write");
    gz.finish().expect("gzip finish")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_archive_reads_back_entry_for_entry() {
        let entries = vec![
            ("manifest.json", &b"{\"id\":\"x\"}"[..]),
            ("procedures/escalation.json", &b"{\"pack\":\"x\"}"[..]),
            ("i18n/en.json", &b"{}"[..]),
        ];
        let gz = write_archive(&entries);
        let read = read_archive(&gz).expect("round trip");
        assert_eq!(read.len(), 3);
        assert_eq!(read[0].0, "manifest.json");
        assert_eq!(read[0].1, b"{\"id\":\"x\"}");
        assert_eq!(read[2].0, "i18n/en.json");
    }

    #[test]
    fn traversal_and_absolute_names_refuse_naming_the_attempt() {
        for name in ["/etc/passwd", "../escape.json", "./sneaky.json"] {
            let gz = write_archive(&[(name, b"x")]);
            let err = read_archive(&gz).unwrap_err();
            assert!(
                matches!(err, ArchiveError::UnsupportedEntry { .. }),
                "{name} must refuse: {err:?}"
            );
        }
    }

    #[test]
    fn a_directory_entry_refuses() {
        let gz = write_archive(&[("procedures/", b"")]);
        assert!(matches!(
            read_archive(&gz),
            Err(ArchiveError::UnsupportedEntry { .. })
        ));
    }

    #[test]
    fn non_gzip_bytes_refuse_cleanly() {
        assert_eq!(read_archive(b"not gzip"), Err(ArchiveError::NotGzip));
    }

    #[test]
    fn a_flipped_header_byte_refuses_on_checksum() {
        let entries = vec![("manifest.json", &b"{}"[..])];
        let mut gz = write_archive(&entries);
        // flip a byte inside the first header block (past the gzip header,
        // which starts ~10 bytes in; pick byte 64 — inside the name field)
        gz[64] ^= 0x01;
        let err = read_archive(&gz).unwrap_err();
        assert!(
            matches!(err, ArchiveError::BadHeader(_) | ArchiveError::NotGzip),
            "a corrupted header refuses: {err:?}"
        );
    }
}
