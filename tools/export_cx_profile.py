#!/usr/bin/env python3
"""Reproduce the bundled profile from the pinned, data-only GARbro catalog.

Run with: uv run --with nrbf tools/export_cx_profile.py Formats.dat OUTPUT.json
This developer tool does not load .NET assemblies or execute serialized objects.
"""
import argparse
import hashlib
import json
from pathlib import Path
import zlib
import nrbf

EXPECTED_SHA256 = "54039fde222592c911536b0bd3f4d931bd580c66afd96eaefece34ea872f2982"
KEYS = ["m_mask", "m_offset", "PrologOrder", "OddBranchOrder", "EvenBranchOrder", "ControlBlock"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("catalog", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    data = args.catalog.read_bytes()
    if hashlib.sha256(data).hexdigest() != EXPECTED_SHA256:
        parser.error("catalog does not match the pinned GARbro revision")
    document = nrbf.loads(zlib.decompress(data[12:]))
    profile = document["SchemeMap"]["XP3"]["KnownSchemes"]["Otome＊Domain"]
    if profile["__class__"] != "GameRes.Formats.KiriKiri.CxEncryption":
        parser.error("unexpected encryption class")
    with args.output.open("x", encoding="utf-8") as out:
        json.dump({key: profile[key] for key in KEYS}, out, indent=2)
        out.write("\n")


if __name__ == "__main__":
    main()
