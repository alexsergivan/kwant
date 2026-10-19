"""Regenerates the binary import fixtures (PNG card, hostile zips). Run: python3 -I make_fixtures.py"""
import base64, io, json, os, struct, zipfile, zlib

HERE = os.path.dirname(os.path.abspath(__file__))

def png_with_text(chunks):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    ihdr = struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)
    idat = zlib.compress(b"\x00\xff\xff\xff")
    out = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr)
    for key, value in chunks:
        out += chunk(b"tEXt", key + b"\x00" + value)
    return out + chunk(b"IDAT", idat) + chunk(b"IEND", b"")

card = {"spec": "chara_card_v3", "spec_version": "3.0", "data": {
    "name": "Lumen", "description": "{{char}} explains astronomy to {{user}} simply.", "personality": "Curious and patient.",
    "first_mes": "Want to look at the sky tonight?", "system_prompt": "Use plain words.", "scenario": "", "mes_example": "",
    "alternate_greetings": [], "tags": [], "creator": "fixture", "character_version": "1", "extensions": {},
    "character_book": {"entries": []}}}
old = dict(card, spec="chara_card_v2")
with open(os.path.join(HERE, "card-v3.png"), "wb") as f:
    f.write(png_with_text([(b"chara", base64.b64encode(json.dumps(old).encode())), (b"ccv3", base64.b64encode(json.dumps(card).encode()))]))

def write_zip(name, entries):
    with zipfile.ZipFile(os.path.join(HERE, "hostile", name), "w", zipfile.ZIP_DEFLATED) as z:
        for path, data in entries:
            z.writestr(zipfile.ZipInfo(path), data, zipfile.ZIP_DEFLATED)

agent = b"---\nname: ok\ndescription: fine\n---\nHello."
# One file that inflates past the 4 MB per-file cap (5 MB of zeros, ~5 KB on disk).
write_zip("zip-bomb-one-file.zip", [("agents/big.md", b"0" * (5 * 1024 * 1024))])
# Many files under 4 MB each that together pass the 50 MB total cap.
write_zip("zip-bomb-total.zip", [(f"agents/a{i}.md", b"0" * (4 * 1024 * 1024 - 10)) for i in range(14)])
write_zip("zip-slip.zip", [("agents/ok.md", agent), ("../../evil.md", agent)])
write_zip("zip-absolute.zip", [("/etc/evil.md", agent)])
write_zip("zip-backslash.zip", [("agents\\..\\..\\evil.md", agent)])
inner = io.BytesIO()
with zipfile.ZipFile(inner, "w") as z:
    z.writestr("agents/ok.md", agent)
write_zip("zip-nested.zip", [("agents/ok.md", agent), ("more.zip", inner.getvalue())])
write_zip("zip-too-many-files.zip", [(f"f{i}.txt", b"x") for i in range(501)])
