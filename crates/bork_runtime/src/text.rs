//! String helpers called from generated code. Strings are `(ptr, len)` byte ranges; `ptr` may be
//! null when `len == 0`, and no helper looks for a terminator.

use std::slice;

/// # Safety
/// For `len > 0`, `ptr` must reference `len` readable bytes.
unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    #[cfg(feature = "debug-runtime")]
    crate::arena::debug::diag::check(ptr as usize, len, false);
    if len == 0 {
        &[]
    } else {
        // SAFETY: The caller guarantees `len` readable bytes at `ptr`.
        unsafe { slice::from_raw_parts(ptr, len) }
    }
}

#[no_mangle]
pub unsafe extern "C" fn bork_string_equal(
    left: *const u8,
    left_len: usize,
    right: *const u8,
    right_len: usize,
) -> u8 {
    // SAFETY: Forwarded from the caller.
    let (left, right) = unsafe { (bytes(left, left_len), bytes(right, right_len)) };
    (left == right) as u8
}

/// Whether `bytes[..len]` is well-formed UTF-8. An empty range is valid.
#[no_mangle]
pub unsafe extern "C" fn bork_utf8_valid(ptr: *const u8, len: usize) -> u8 {
    // SAFETY: Forwarded from the caller.
    let bytes = unsafe { bytes(ptr, len) };
    std::str::from_utf8(bytes).is_ok() as u8
}

/// FNV-1a 64. Internal ABI for future hash maps; there is no Bork builtin for it yet.
#[no_mangle]
pub unsafe extern "C" fn bork_string_hash(ptr: *const u8, len: usize) -> u64 {
    // SAFETY: Forwarded from the caller.
    let bytes = unsafe { bytes(ptr, len) };
    bytes.iter().fold(14695981039346656037, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(1099511628211)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::null;

    fn equal(a: &[u8], b: &[u8]) -> bool {
        unsafe { bork_string_equal(a.as_ptr(), a.len(), b.as_ptr(), b.len()) == 1 }
    }

    fn hash(a: &[u8]) -> u64 {
        unsafe { bork_string_hash(a.as_ptr(), a.len()) }
    }

    #[test]
    fn empty_inputs_may_be_null() {
        unsafe {
            assert_eq!(bork_string_equal(null(), 0, null(), 0), 1);
            assert_eq!(bork_string_equal(null(), 0, b"a".as_ptr(), 1), 0);
            assert_eq!(bork_string_hash(null(), 0), 14695981039346656037);
        }
    }

    #[test]
    fn equal_compares_bytes_not_addresses() {
        let (a, b) = (b"GET /".to_vec(), b"GET /".to_vec());
        assert_ne!(a.as_ptr(), b.as_ptr());
        assert!(equal(&a, &b));
        assert!(!equal(b"GET", b"GET "));
        assert!(!equal(b"GET", b"GEt"));
    }

    #[test]
    fn embedded_nul_and_utf8_are_ordinary_bytes() {
        assert!(equal(b"a\0b", b"a\0b"));
        assert!(!equal(b"a\0b", b"a\0c"));
        assert!(!equal(b"a\0", b"a"));
        assert!(equal("ą".as_bytes(), &[196, 133]));
    }

    #[test]
    fn utf8_validation_rejects_malformed_sequences() {
        let valid = |a: &[u8]| unsafe { bork_utf8_valid(a.as_ptr(), a.len()) == 1 };
        assert!(valid(b""));
        assert!(valid(b"GET\0x"));
        assert!(valid("zażółć".as_bytes()));
        assert!(!valid(&[0xC4]));
        assert!(!valid(&[0x80]));
        assert!(!valid(&[0xC0, 0xAF]));
        assert!(!valid(&[0xED, 0xA0, 0x80]));
        assert!(unsafe { bork_utf8_valid(null(), 0) == 1 });
    }

    #[test]
    fn hash_matches_fnv1a_vectors_and_ignores_address() {
        assert_eq!(hash(b""), 14695981039346656037);
        assert_eq!(hash(b"a"), 12638187200555641996);
        assert_eq!(hash(b"a\0"), hash(&b"a\0".to_vec()));
        assert_ne!(hash(b"a\0"), hash(b"a"));
    }
}
