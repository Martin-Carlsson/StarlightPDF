//! Published test vectors.

use core::slice;
use core::str;

use super::aes::Aes;
use super::md5::Md5;
use super::rc4::Rc4;
use super::sha2::{sha256, sha384, sha512};

/// FIPS 197, Appendix C.1 (AES-128), C.3 (AES-256): key, plaintext, ciphertext.
#[test]
fn aes_fips_197() {
    for (key, plain, cipher) in [
        (
            "000102030405060708090a0b0c0d0e0f",
            "00112233445566778899aabbccddeeff",
            "69c4e0d86a7b0430d8cdb78070b4c55a",
        ),
        (
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "00112233445566778899aabbccddeeff",
            "8ea2b7ca516745bfeafc49904b496089",
        ),
    ] {
        let aes = hex(key).and_then(|bytes| Aes::new(&bytes));
        let mut encrypted = block(plain);
        let mut decrypted = block(cipher);
        if let (Some(cipher_key), Some(enc), Some(dec)) =
            (aes.as_ref(), encrypted.as_mut(), decrypted.as_mut())
        {
            cipher_key.encrypt_cbc(&[0; 16], enc);
            cipher_key.decrypt_cbc(&mut [0; 16], slice::from_mut(dec));
        }
        assert_eq!(encrypted, block(cipher), "encrypting with key {key}");
        assert_eq!(decrypted, block(plain), "decrypting with key {key}");
    }
}

/// The 16 bytes of 32 hex digits.
fn block(digits: &str) -> Option<[u8; 16]> {
    hex(digits)?.try_into().ok()
}

/// The bytes of pairs of hex digits.
fn hex(digits: &str) -> Option<Vec<u8>> {
    digits
        .as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

/// RFC 1321, A.5: the test suite, hashed whole and a byte at a time.
#[test]
fn md5_rfc_1321() {
    for (message, digest) in [
        ("", "d41d8cd98f00b204e9800998ecf8427e"),
        ("a", "0cc175b9c0f1b6a831c399e269772661"),
        ("abc", "900150983cd24fb0d6963f7d28e17f72"),
        ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
        (
            "abcdefghijklmnopqrstuvwxyz",
            "c3fcd3d76192e4007dfb496cca67e13b",
        ),
        (
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
            "d174ab98d277d9f5a5611c2c9f419d9f",
        ),
        (
            "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
            "57edf4a22be3c955ac49da2e2107b67a",
        ),
    ] {
        let mut whole = Md5::default();
        whole.update(message.as_bytes());
        assert_eq!(
            Some(whole.finish().to_vec()),
            hex(digest),
            "MD5 of \"{message}\""
        );
        let mut bytewise = Md5::default();
        for byte in message.as_bytes() {
            bytewise.update(slice::from_ref(byte));
        }
        assert_eq!(
            Some(bytewise.finish().to_vec()),
            hex(digest),
            "MD5 of \"{message}\" a byte at a time"
        );
    }
}

/// RFC 6229, 2: keystream at offsets 0, 16, 1536 and 4096 of the 40- and 128-bit keys.
#[test]
fn rc4_rfc_6229() {
    for (key, vectors) in [
        (
            "0102030405",
            [
                (0, "b2396305f03dc027ccc3524a0a1118a8"),
                (16, "6982944f18fc82d589c403a47a0d0919"),
                (1536, "d8729db41882259bee4f825325f5a130"),
                (4096, "ff25b58995996707e51fbdf08b34d875"),
            ],
        ),
        (
            "0102030405060708090a0b0c0d0e0f10",
            [
                (0, "9ac7cc9a609d1ef7b2932899cde41b97"),
                (16, "5248c4959014126a6e8a84f11d1a9e1c"),
                (1536, "ffa0b514647ec04f6306b892ae661181"),
                (4096, "a36a4c301ae8ac13610ccbc12256cacc"),
            ],
        ),
    ] {
        let mut stream = vec![0_u8; 4112];
        if let Some(mut rc4) = hex(key).and_then(|bytes| Rc4::new(&bytes)) {
            let (head, tail) = stream.split_at_mut(1536);
            rc4.apply(head);
            rc4.apply(tail);
        }
        for (offset, expected) in vectors {
            assert_eq!(
                stream
                    .get(offset..)
                    .and_then(|rest| rest.get(..16))
                    .map(<[u8]>::to_vec),
                hex(expected),
                "key {key} at offset {offset}"
            );
        }
    }
}

/// FIPS 180-4, 6.2; NIST's SHA-256 examples: "abc" and the 448-bit message.
#[test]
fn sha256_fips_180_4() {
    for (message, digest) in [
        (
            "abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
    ] {
        assert_eq!(
            Some(sha256(message.as_bytes()).to_vec()),
            hex(digest),
            "SHA-256 of {message}"
        );
    }
}

/// FIPS 180-4, 6.5; NIST's SHA-384 examples: "abc" and the 896-bit message.
#[test]
fn sha384_fips_180_4() {
    for (message, digest) in [
        (
            "abc",
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7",
        ),
        (
            "abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu",
            "09330c33f71147e83d192fc782cd1b4753111b173b3b05d22fa08086e3b0f712fcc7c71a557e2db966c3e9fa91746039",
        ),
    ] {
        assert_eq!(
            Some(sha384(message.as_bytes()).to_vec()),
            hex(digest),
            "SHA-384 of {message}"
        );
    }
}

/// FIPS 180-4, 6.4; NIST's SHA-512 examples: "abc" and the 896-bit message.
#[test]
fn sha512_fips_180_4() {
    for (message, digest) in [
        (
            "abc",
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
        ),
        (
            "abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu",
            "8e959b75dae313da8cf4f72814fc143f8f7779c6eb9f7fa17299aeadb6889018501d289e4900f7e4331b99dec4b5433ac7d329eeb6dd26545e96e55b874be909",
        ),
    ] {
        assert_eq!(
            Some(sha512(message.as_bytes()).to_vec()),
            hex(digest),
            "SHA-512 of {message}"
        );
    }
}
