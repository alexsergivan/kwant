#!/usr/bin/env python3
"""Checks a Tauri updater signature (minisign, base64-wrapped) the way the Mac app will.

    python3 scripts/minisign_verify.py --pubkey <base64 pubkey | file> --file kwant.app.tar.gz \\
        --signature kwant.app.tar.gz.sig [--version 0.1.2]

Verifies the file signature (Ed25519 over the file's BLAKE2b-512 for prehashed "ED" signatures),
the global signature over the trusted comment, that both keys match, and with --version that the
trusted comment carries `version:<version>`, which the app requires (requireSignedVersion), so a
release signed with the wrong key or without the version fails CI instead of every installed app.
Needs PyNaCl (pip install pynacl).
"""

import argparse
import base64
import hashlib
import os
import sys

from nacl.exceptions import BadSignatureError
from nacl.signing import VerifyKey


def _unwrap(text: str) -> list:
    """Tauri stores minisign files base64-encoded; returns their lines."""
    text = text.strip()
    if not text.startswith("untrusted comment:"):
        text = base64.b64decode(text).decode("utf-8")
    return [line for line in text.splitlines() if line.strip()]


def parse_pubkey(text: str):
    raw = base64.b64decode(_unwrap(text)[1])
    if len(raw) != 42 or raw[:2] != b"Ed":
        raise ValueError("not a minisign Ed25519 public key")
    return raw[2:10], raw[10:]


def verify(pubkey_text: str, data: bytes, sig_text: str, version=None) -> str:
    """Raises ValueError unless the signature is good; returns the trusted comment."""
    key_id, pk = parse_pubkey(pubkey_text)
    lines = _unwrap(sig_text)
    if len(lines) < 4 or not lines[2].startswith("trusted comment: "):
        raise ValueError("malformed signature file")
    sig = base64.b64decode(lines[1])
    if len(sig) != 74 or sig[:2] not in (b"Ed", b"ED"):
        raise ValueError("not a minisign Ed25519 signature")
    if sig[2:10] != key_id:
        raise ValueError(
            f"signed with key {sig[2:10][::-1].hex().upper()}, the app trusts {key_id[::-1].hex().upper()}"
        )
    message = hashlib.blake2b(data, digest_size=64).digest() if sig[:2] == b"ED" else data
    trusted = lines[2][len("trusted comment: "):]
    key = VerifyKey(pk)
    try:
        key.verify(message, sig[10:])
    except BadSignatureError:
        raise ValueError("the file doesn't match its signature")
    try:
        key.verify(sig[10:] + trusted.encode("utf-8"), base64.b64decode(lines[3]))
    except BadSignatureError:
        raise ValueError("the trusted comment doesn't match its signature")
    if version is not None:
        signed = [f[len("version:"):] for f in trusted.split("\t") if f.startswith("version:")]
        if signed != [version]:
            raise ValueError(f"trusted comment carries version {signed or 'none'}, expected {version}: {trusted}")
    return trusted


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--pubkey", required=True, help="base64 public key, or a file holding it")
    ap.add_argument("--file", required=True)
    ap.add_argument("--signature", required=True)
    ap.add_argument("--version")
    args = ap.parse_args(argv)
    pubkey = open(args.pubkey).read() if os.path.isfile(args.pubkey) else args.pubkey
    with open(args.file, "rb") as f:
        data = f.read()
    try:
        trusted = verify(pubkey, data, open(args.signature).read(), args.version)
    except (ValueError, IndexError) as e:
        print(f"minisign_verify: {e}", file=sys.stderr)
        return 1
    print(f"minisign_verify: good signature; trusted comment: {trusted}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
