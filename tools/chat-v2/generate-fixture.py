#!/usr/bin/env python3
"""Independent extended-chat vector using Python HMAC and system libsodium.

All inputs are public deterministic test values. Rust tests consume the checked
fixtures without needing Python or libsodium. Run from any working directory.
"""
import ctypes
import ctypes.util
import hashlib
import hmac
from pathlib import Path


def array(*fields):
    assert 0 < len(fields) < 16
    return bytes([0x90 + len(fields)]) + b"".join(fields)


def blob(value):
    assert len(value) < 256
    return bytes([0xC4, len(value)]) + value


def entity(kind, tag):
    return blob(bytes([kind]) + bytes([tag]) * 32)


def main():
    library = ctypes.util.find_library("sodium")
    if not library:
        raise SystemExit("libsodium is required to regenerate this independent fixture")
    sodium = ctypes.CDLL(library)
    assert sodium.sodium_init() >= 0
    sodium.crypto_secretbox_easy.argtypes = [
        ctypes.c_void_p, ctypes.c_char_p, ctypes.c_ulonglong,
        ctypes.c_char_p, ctypes.c_char_p,
    ]
    sodium.crypto_secretbox_easy.restype = ctypes.c_int
    # Format 2, host/team/channel, actor, operation, role/generation, reply action.
    member_role = array(b"\x01", b"\x81\xa1\x30\x00")
    context = array(
        b"\x02", entity(2, 1), entity(3, 2), blob(bytes([3]) * 16),
        entity(1, 4), blob(bytes([5]) * 16), array(member_role, b"\x01"),
        array(b"\x02", blob(bytes([6]) * 16), blob(bytes([7]) * 16), blob(bytes([8]) * 16)),
    )
    key_domain = hashlib.sha256(b"foks.chat.v2.context-key").digest()[:8]
    body_domain = hashlib.sha256(b"foks.chat.v2.encrypted-body").digest()[:8]
    key = hmac.new(bytes([9]) * 32, key_domain + context, "sha512_256").digest()
    partial_nonce = bytes([10]) * 16
    body = b"original text"
    ciphertext = ctypes.create_string_buffer(len(body) + 16)
    assert sodium.crypto_secretbox_easy(ciphertext, body, len(body), body_domain + partial_nonce, key) == 0
    root = Path(__file__).resolve().parents[2] / "crates/foks-crypto/tests/fixtures/chat-v2"
    root.mkdir(parents=True, exist_ok=True)
    for name, contents in {
        "context.snowp": context,
        "key.bin": key,
        "ciphertext.bin": ciphertext.raw,
    }.items():
        (root / name).write_bytes(contents)


if __name__ == "__main__":
    main()
