//! Self-contained program bundles (v0.7): stapling a program's assets onto
//! the interpreter to make one fixed, runnable binary.
//!
//! `socrates build DIR` walks a program's directory, packs every file into a
//! payload, and appends `payload || u64(len) || MAGIC` to a copy of the
//! `socrates` executable. When that stapled binary starts, [`read_self`] finds
//! the trailer, [`Bundle::extract_to`] unpacks the files into a scratch
//! directory, and the normal runner takes over from there — so imports,
//! `fs.*`, and `worker.spawn` all resolve against real files with no
//! special-casing anywhere in the loader or the VM.
//!
//! The format is deliberately dumb and dependency-free: little-endian
//! length-prefixed records. Reading its own tail costs one 16-byte read for
//! an ordinary (un-stapled) `socrates`, so the launcher path is free until a
//! bundle is actually present.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Trailer sentinel; also the format version (bump the trailing digit on an
/// incompatible layout change). Chosen to be unlikely in a binary's tail.
const MAGIC: &[u8; 8] = b"SOCRZOO1";

/// On macOS the payload can't be appended (a Mach-O with data past
/// `__LINKEDIT` fails code signing, and Apple Silicon won't run unsigned), so
/// it is linked in as a section instead — `ld -sectcreate __DATA __socrateszoo
/// payload.bin` — and read back out of the image by [`macho_section`]. Names
/// live in a fixed 16-byte, NUL-padded field.
const MACHO_SECTION_NAME: &[u8] = b"__socrateszoo";

/// An unpacked program: the entry file's bundle-relative path plus every
/// file's bundle-relative path and contents.
pub struct Bundle {
    pub entry: String,
    pub files: Vec<(String, Vec<u8>)>,
}

/// Serialize a payload from an entry path and a set of `(relpath, bytes)`.
/// Paths are stored with `/` separators regardless of host.
pub fn payload(entry: &str, files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    put_str(&mut out, entry);
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for (path, data) in files {
        put_str(&mut out, path);
        out.extend_from_slice(&(data.len() as u64).to_le_bytes());
        out.extend_from_slice(data);
    }
    out
}

/// Staple a payload onto launcher bytes, producing a runnable binary image.
pub fn staple(launcher: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(launcher.len() + payload.len() + 16);
    out.extend_from_slice(launcher);
    out.extend_from_slice(payload);
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(MAGIC);
    out
}

/// Read the bundle stapled onto the currently running executable, if any.
/// Returns `None` for an ordinary `socrates` (no trailer) and on any read or
/// parse trouble — a malformed tail must never keep the plain CLI from
/// running.
pub fn read_self() -> Option<Bundle> {
    let exe = std::env::current_exe().ok()?;
    // Fast path: the trailer is the exact tail (ELF and PE tolerate appended
    // data, so the magic is the last 8 bytes). One 16-byte read for an
    // ordinary `socrates`.
    if let Ok(Some(b)) = read_from(&exe) {
        return Some(b);
    }
    let data = std::fs::read(&exe).ok()?;
    // macOS: the payload rides in a Mach-O `__socrateszoo` section, not the tail.
    if let Some(b) = macho_section(&data) {
        return Some(b);
    }
    // Fallback: tolerate trailing bytes after the trailer (e.g. a code
    // signature appended past our payload) by scanning the whole image
    // backward for the magic.
    scan(&data)
}

/// Find the payload in a `__socrateszoo` section of a 64-bit little-endian
/// Mach-O (our only macOS target is arm64), reading it straight from the file
/// image by its recorded offset — no ASLR/slide concerns. Returns `None` for
/// any non-Mach-O binary or a Mach-O without the section, so ELF/PE and an
/// ordinary `socrates` fall through untouched.
fn macho_section(data: &[u8]) -> Option<Bundle> {
    const MH_MAGIC_64: u32 = 0xFEED_FACF;
    const LC_SEGMENT_64: u32 = 0x19;
    let u32at = |o: usize| data.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let u64at = |o: usize| data.get(o..o + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()));

    if u32at(0)? != MH_MAGIC_64 {
        return None;
    }
    let ncmds = u32at(16)? as usize;
    let mut off = 32; // sizeof(mach_header_64)
    for _ in 0..ncmds {
        let cmd = u32at(off)?;
        let cmdsize = u32at(off + 4)? as usize;
        if cmdsize < 8 {
            return None;
        }
        if cmd == LC_SEGMENT_64 {
            // segment_command_64: nsects is at +64, section_64 records at +72.
            let nsects = u32at(off + 64)? as usize;
            for s in 0..nsects {
                let sect = off + 72 + s * 80; // sizeof(section_64) == 80
                let name = data.get(sect..sect + 16)?;
                if name16_eq(name, MACHO_SECTION_NAME) {
                    let size = u64at(sect + 40)? as usize; // section_64.size
                    let foff = u32at(sect + 48)? as usize; // section_64.offset
                    return parse(data.get(foff..foff.checked_add(size)?)?);
                }
            }
        }
        off = off.checked_add(cmdsize)?;
    }
    None
}

/// Compare a fixed 16-byte, NUL-padded Mach-O name field against `name`.
fn name16_eq(field: &[u8], name: &[u8]) -> bool {
    name.len() <= 16
        && field.len() == 16
        && &field[..name.len()] == name
        && field[name.len()..].iter().all(|&b| b == 0)
}

/// Find the stapled bundle by scanning `data` backward for the trailer magic,
/// tolerating trailing bytes after it (e.g. a Mach-O code signature). Returns
/// the payload of the last magic whose length prefix yields a parseable
/// bundle.
fn scan(data: &[u8]) -> Option<Bundle> {
    if data.len() < 16 {
        return None;
    }
    let mut p = data.len() - 8;
    loop {
        if &data[p..p + 8] == MAGIC && p >= 8 {
            let len = u64::from_le_bytes(data[p - 8..p].try_into().unwrap()) as usize;
            if let Some(start) = (p - 8).checked_sub(len) {
                if let Some(b) = parse(&data[start..p - 8]) {
                    return Some(b);
                }
            }
        }
        if p == 0 {
            return None;
        }
        p -= 1;
    }
}

fn read_from(path: &Path) -> io::Result<Option<Bundle>> {
    let mut f = std::fs::File::open(path)?;
    let total = f.seek(SeekFrom::End(0))?;
    if total < 16 {
        return Ok(None);
    }
    // The last 16 bytes are `[u64 payload_len][MAGIC]`.
    f.seek(SeekFrom::End(-16))?;
    let mut trailer = [0u8; 16];
    f.read_exact(&mut trailer)?;
    if &trailer[8..16] != MAGIC {
        return Ok(None);
    }
    let len = u64::from_le_bytes(trailer[0..8].try_into().unwrap());
    // A plausible payload sits entirely within the file, before the trailer.
    if len == 0 || len + 16 > total {
        return Ok(None);
    }
    f.seek(SeekFrom::Start(total - 16 - len))?;
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf)?;
    Ok(parse(&buf))
}

/// Parse a payload blob into a [`Bundle`]. Returns `None` on any truncation
/// or length that runs past the buffer.
pub fn parse(buf: &[u8]) -> Option<Bundle> {
    let mut c = Cursor { buf, pos: 0 };
    let entry = c.take_str()?;
    let count = c.take_u32()? as usize;
    let mut files = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let path = c.take_str()?;
        let len = c.take_u64()? as usize;
        let data = c.take(len)?.to_vec();
        files.push((path, data));
    }
    Some(Bundle { entry, files })
}

impl Bundle {
    /// Write every bundled file under `dir`, creating parent directories as
    /// needed, and return the absolute path of the extracted entry file.
    /// Rejects paths that would escape `dir` (absolute or `..`).
    pub fn extract_to(&self, dir: &Path) -> io::Result<PathBuf> {
        for (rel, data) in &self.files {
            let target = safe_join(dir, rel).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, format!("unsafe bundle path `{rel}`"))
            })?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&target, data)?;
        }
        safe_join(dir, &self.entry)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unsafe bundle entry path"))
    }
}

/// Join a `/`-separated bundle-relative path onto `base`, refusing anything
/// that is absolute or climbs out with `..`.
fn safe_join(base: &Path, rel: &str) -> Option<PathBuf> {
    let mut out = base.to_path_buf();
    for seg in rel.split('/') {
        match seg {
            "" | "." => continue,
            ".." => return None,
            s => {
                if s.contains('\\') || Path::new(s).is_absolute() {
                    return None;
                }
                out.push(s);
            }
        }
    }
    Some(out)
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let slice = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }
    fn take_u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn take_u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn take_str(&mut self) -> Option<String> {
        let n = self.take_u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let files = vec![
            ("main.soc".to_string(), b"println(\"hi\");".to_vec()),
            ("lib/util.soc".to_string(), b"pub fn f() -> Int { 1 }".to_vec()),
            ("data.bin".to_string(), vec![0u8, 1, 2, 255, 254]),
        ];
        let blob = payload("main.soc", &files);
        let b = parse(&blob).expect("parses");
        assert_eq!(b.entry, "main.soc");
        assert_eq!(b.files, files);
    }

    #[test]
    fn staple_then_read_tail() {
        let launcher = b"pretend-this-is-an-executable-image".to_vec();
        let files = vec![("main.soc".to_string(), b"1".to_vec())];
        let img = staple(&launcher, &payload("main.soc", &files));
        // The trailer must be exactly the last 16 bytes.
        assert_eq!(&img[img.len() - 8..], MAGIC);
        // And the payload parses back out of the middle.
        let plen = u64::from_le_bytes(img[img.len() - 16..img.len() - 8].try_into().unwrap());
        let start = img.len() - 16 - plen as usize;
        let b = parse(&img[start..img.len() - 16]).expect("parses");
        assert_eq!(b.files, files);
    }

    #[test]
    fn plain_binary_has_no_bundle() {
        assert!(parse(b"not a payload at all").is_none());
    }

    #[test]
    fn scan_finds_magic_before_a_trailing_signature() {
        let files = vec![("main.soc".to_string(), b"println(1);".to_vec())];
        let mut img = staple(b"mach-o-image", &payload("main.soc", &files));
        // Simulate a code signature appended after our trailer.
        img.extend_from_slice(b"\x00fake code signature blob\xff\xfe");
        let b = scan(&img).expect("scan locates the payload past trailing bytes");
        assert_eq!(b.files, files);
    }

    #[test]
    fn scan_ignores_plain_binaries() {
        assert!(scan(b"an ordinary executable with no bundle at all").is_none());
    }

    #[test]
    fn macho_section_reads_payload() {
        let files = vec![("main.soc".to_string(), b"println(42);".to_vec())];
        let pl = payload("main.soc", &files);
        // Minimal 64-bit Mach-O: header + one __DATA segment with one
        // __socrateszoo section whose file offset points past the load commands.
        let cmdsize = 72 + 80usize; // segment_command_64 + one section_64
        let payload_off = 32 + cmdsize;
        let mut m = vec![0u8; payload_off];
        m[0..4].copy_from_slice(&0xFEED_FACFu32.to_le_bytes()); // magic MH_MAGIC_64
        m[16..20].copy_from_slice(&1u32.to_le_bytes()); // ncmds
        m[20..24].copy_from_slice(&(cmdsize as u32).to_le_bytes()); // sizeofcmds
        let c = 32; // LC_SEGMENT_64
        m[c..c + 4].copy_from_slice(&0x19u32.to_le_bytes()); // cmd
        m[c + 4..c + 8].copy_from_slice(&(cmdsize as u32).to_le_bytes()); // cmdsize
        m[c + 8..c + 14].copy_from_slice(b"__DATA"); // segname
        m[c + 64..c + 68].copy_from_slice(&1u32.to_le_bytes()); // nsects
        let s = c + 72; // section_64
        m[s..s + 13].copy_from_slice(b"__socrateszoo"); // sectname (13 bytes, within the 16-byte field)
        m[s + 16..s + 22].copy_from_slice(b"__DATA"); // segname
        m[s + 40..s + 48].copy_from_slice(&(pl.len() as u64).to_le_bytes()); // size
        m[s + 48..s + 52].copy_from_slice(&(payload_off as u32).to_le_bytes()); // offset
        m.extend_from_slice(&pl);

        let b = macho_section(&m).expect("reads the __socrateszoo section");
        assert_eq!(b.entry, "main.soc");
        assert_eq!(b.files, files);
    }

    #[test]
    fn macho_section_ignores_non_macho() {
        assert!(macho_section(b"\x7fELF not a mach-o at all, longer than a header").is_none());
        // A Mach-O with no __socrateszoo section (ncmds = 0).
        let mut m = vec![0u8; 32];
        m[0..4].copy_from_slice(&0xFEED_FACFu32.to_le_bytes());
        assert!(macho_section(&m).is_none());
    }

    #[test]
    fn rejects_escaping_paths() {
        let dir = std::env::temp_dir();
        assert!(safe_join(&dir, "../evil").is_none());
        assert!(safe_join(&dir, "a/../../evil").is_none());
        assert!(safe_join(&dir, "ok/nested/file.txt").is_some());
    }
}
