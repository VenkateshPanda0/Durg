//! Integrity checks for the bootloader binaries that go into the boot image.
//!
//! Limine is part of the trusted computing base: it runs before the kernel and
//! hands over the memory map. A git tag can be moved, so the build pins the
//! **commit** (see `LIMINE_COMMIT` in `main.rs`) and checks the SHA-256 of every
//! Limine file copied into the image against the digests written down here. The
//! check fails closed: a missing file or a different digest is an error, never a
//! reason to carry on or to fetch something else.
//!
//! SHA-256 is implemented here (FIPS 180-4) so the build tool needs no
//! dependencies; the tests below use the published test vectors.

use std::fs;
use std::path::Path;

/// One pinned file: its path relative to the Limine directory and the lowercase
/// hex SHA-256 of its contents.
pub type Pin = (&'static str, &'static str);

/// SHA-256 of `data`, as 32 bytes.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    // Pad: 0x80, zeros, then the bit length as a big-endian u64, to a multiple of 64.
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in message.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (state, add) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *state = state.wrapping_add(add);
        }
    }
    let mut out = [0u8; 32];
    for (chunk, word) in out.chunks_exact_mut(4).zip(h) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Checks every pinned file under `dir`. Fails closed: the first missing,
/// unreadable or different file is an error that says which file, what was
/// expected and what was found.
pub fn verify(dir: &Path, pins: &[Pin]) -> Result<(), String> {
    for (name, expected) in pins {
        let path = dir.join(name);
        let data = fs::read(&path).map_err(|e| {
            format!(
                "cannot read pinned file {}: {e}; delete {} and rebuild to fetch it again",
                path.display(),
                dir.display()
            )
        })?;
        let found = hex(&sha256(&data));
        if found != *expected {
            return Err(format!(
                "{} does not match its pinned SHA-256\n  expected {expected}\n  found    {found}\n\
                 The file was changed after it was fetched, or the pin is out of date. Nothing was \
                 built from it. If you did not change it, delete {} and rebuild; if the digest \
                 still differs, stop and treat the bootloader source as untrusted.",
                path.display(),
                dir.display()
            ));
        }
    }
    Ok(())
}

/// Sets the modification (and access) time of every file and directory under
/// `root`, and of `root` itself, to `time`. Children are done before their parent,
/// because creating or changing a child would otherwise move the parent's time.
pub fn set_tree_mtime(root: &Path, time: std::time::SystemTime) -> Result<(), String> {
    let times = fs::FileTimes::new().set_accessed(time).set_modified(time);
    fn walk(path: &Path, times: fs::FileTimes) -> Result<(), String> {
        let meta =
            fs::symlink_metadata(path).map_err(|e| format!("stat {}: {e}", path.display()))?;
        if meta.is_dir() {
            let mut entries: Vec<_> = fs::read_dir(path)
                .map_err(|e| format!("read {}: {e}", path.display()))?
                .collect::<Result<_, _>>()
                .map_err(|e| format!("read {}: {e}", path.display()))?;
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                walk(&entry.path(), times)?;
            }
        }
        // A directory can be opened read-only to change its times.
        let file = fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
        file.set_times(times)
            .map_err(|e| format!("set times on {}: {e}", path.display()))
    }
    walk(root, times)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("durg-pin-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sha256_matches_the_published_test_vectors() {
        // FIPS 180-4 / NIST examples.
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // One million 'a' (crosses many blocks).
        assert_eq!(
            hex(&sha256(&vec![b'a'; 1_000_000])),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn sha256_handles_the_padding_boundaries() {
        // 55, 56 and 64 bytes are where the length field changes block.
        assert_eq!(
            hex(&sha256(&[b'a'; 55])),
            "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318"
        );
        assert_eq!(
            hex(&sha256(&[b'a'; 56])),
            "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a"
        );
        assert_eq!(
            hex(&sha256(&[b'a'; 64])),
            "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb"
        );
    }

    #[test]
    fn set_tree_mtime_fixes_every_file_and_directory() {
        let dir = temp_dir("mtime");
        fs::create_dir_all(dir.join("sub/deeper")).unwrap();
        fs::write(dir.join("a.txt"), b"a").unwrap();
        fs::write(dir.join("sub/b.txt"), b"b").unwrap();
        fs::write(dir.join("sub/deeper/c.txt"), b"c").unwrap();
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        set_tree_mtime(&dir, when).unwrap();
        for path in [
            "",
            "a.txt",
            "sub",
            "sub/b.txt",
            "sub/deeper",
            "sub/deeper/c.txt",
        ] {
            let modified = fs::metadata(dir.join(path)).unwrap().modified().unwrap();
            assert_eq!(modified, when, "mtime of {path:?}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_clean_directory_passes() {
        let dir = temp_dir("clean");
        fs::write(dir.join("a.bin"), b"abc").unwrap();
        let pins: &[Pin] = &[(
            "a.bin",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )];
        assert_eq!(verify(&dir, pins), Ok(()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tampered_file_is_refused_and_named() {
        let dir = temp_dir("tampered");
        fs::write(dir.join("a.bin"), b"abd").unwrap();
        let pins: &[Pin] = &[(
            "a.bin",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )];
        let error = verify(&dir, pins).unwrap_err();
        assert!(error.contains("a.bin"), "{error}");
        assert!(
            error.contains("does not match its pinned SHA-256"),
            "{error}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_is_refused() {
        let dir = temp_dir("missing");
        let pins: &[Pin] = &[("gone.bin", "00")];
        let error = verify(&dir, pins).unwrap_err();
        assert!(error.contains("cannot read pinned file"), "{error}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_first_bad_file_stops_the_check() {
        let dir = temp_dir("first");
        fs::write(dir.join("good.bin"), b"abc").unwrap();
        fs::write(dir.join("bad.bin"), b"zzz").unwrap();
        let pins: &[Pin] = &[
            (
                "good.bin",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            ("bad.bin", "00"),
        ];
        assert!(verify(&dir, pins).unwrap_err().contains("bad.bin"));
        let _ = fs::remove_dir_all(&dir);
    }
}
