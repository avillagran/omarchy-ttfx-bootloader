#!/usr/bin/env python3
"""Add or remove the Bootloader block in the user's Omarchy menu extension.

The extension is JSONC that other tools also edit and mark with their own
comments, so it is never re-serialized: only the lines between our markers
change, plus the style.bootloader* entries the 2026-09 installer wrote. The
result must still parse the way Omarchy's menu reads it (whole-line //
comments removed, trailing commas allowed) and keep every other entry.

Usage: menu-extension.py add FILE BLOCK | menu-extension.py remove FILE
"""

import json
import os
from pathlib import Path
import re
import sys
import tempfile

START = "// omarchy-ttfx-bootloader:start"
END = "// omarchy-ttfx-bootloader:end"
LEGACY_PREFIX = "style.bootloader"
LEGACY_MULTILINE = re.compile(r'^  "style\.bootloader(?:\.[a-z-]+)?": \{\n(?:    .*\n)*?  \},?[ \t]*\n', re.M)
LEGACY_ONELINE = re.compile(r'^[ \t]*"style\.bootloader(?:\.[a-z-]+)?"[ \t]*:[ \t]*\{.*\},?[ \t]*\n', re.M)


def fail(message):
    print(f"menu-extension: {message}", file=sys.stderr)
    sys.exit(1)


def parse(text):
    """Parse the way Omarchy's MenuModel.stripJsonc does."""
    stripped = re.sub(r"^\s*//[^\n]*(\n|$)", "", text, flags=re.M)
    stripped = re.sub(r",(\s*[}\]])", r"\1", stripped)
    if not stripped.strip():
        return {}
    value = json.loads(stripped)
    if not isinstance(value, dict):
        raise ValueError("the menu extension is not an object")
    return value


def without_ours(text):
    lines = text.splitlines(keepends=True)
    kept = []
    removed = []
    inside = False
    for line in lines:
        marker = line.strip()
        if marker == START:
            if inside:
                fail("nested Bootloader block markers")
            inside = True
        elif marker == END:
            if not inside:
                fail("Bootloader block end marker without a start")
            inside = False
        elif inside:
            removed.append(line)
        else:
            kept.append(line)
    if inside:
        fail("unterminated Bootloader block")
    text = "".join(kept)
    text = LEGACY_MULTILINE.sub("", text)
    return LEGACY_ONELINE.sub("", text), "".join(removed)


def insert_block(text, block):
    if not text.strip():
        return "{\n" + block + "}\n"
    offset = 0
    for line in text.splitlines(keepends=True):
        stripped = line.lstrip()
        if not stripped.strip() or stripped.startswith("//"):
            offset += len(line)
            continue
        if not stripped.startswith("{"):
            fail("the menu extension does not start with an object")
        brace = offset + len(line) - len(stripped)
        rest = text[brace + 1:]
        return text[:brace + 1] + "\n" + block + (rest[1:] if rest.startswith("\n") else rest)
    fail("the menu extension has no object")


def write_atomically(path, text):
    mode = path.stat().st_mode & 0o7777 if path.exists() else 0o644
    path.parent.mkdir(parents=True, exist_ok=True)
    handle, temporary = tempfile.mkstemp(dir=path.parent, prefix=f".{path.name}.")
    try:
        with os.fdopen(handle, "w") as stream:
            stream.write(text)
        os.chmod(temporary, mode)
        os.replace(temporary, path)
    except BaseException:
        os.unlink(temporary)
        raise


def main(argv):
    if len(argv) == 4 and argv[1] == "add":
        action, path, block_path = argv[1], Path(argv[2]), Path(argv[3])
    elif len(argv) == 3 and argv[1] == "remove":
        action, path, block_path = argv[1], Path(argv[2]), None
    else:
        fail("usage: menu-extension.py add FILE BLOCK | remove FILE")

    if path.is_symlink():
        fail(f"refusing to edit a symlinked menu extension: {path}")
    original = path.read_text() if path.exists() else ""
    try:
        before = parse(original)
    except ValueError as error:
        fail(f"cannot parse {path}, leaving it untouched: {error}")
    text, previous_block = without_ours(original)
    try:
        previous = parse("{\n" + previous_block + "}\n")
    except ValueError as error:
        fail(f"cannot parse the current Bootloader block, leaving the file untouched: {error}")
    others = {
        key: value for key, value in before.items()
        if not key.startswith(LEGACY_PREFIX) and key not in previous
    }
    ours = {}
    if action == "add":
        block_text = block_path.read_text()
        ours = parse("{\n" + block_text + "}\n")
        if not ours or any(not key.startswith("appearance.bootloader") for key in ours):
            fail("the Bootloader block must only hold appearance.bootloader entries")
        block = f"  {START}\n{block_text}  {END}\n"
        text = insert_block(text, block)

    try:
        after = parse(text)
    except ValueError as error:
        fail(f"the edited menu extension would not parse, leaving it untouched: {error}")
    expected = {key: value for key, value in others.items() if key not in ours}
    expected.update(ours)
    if after != expected:
        fail("the edit would change entries it does not own, leaving the file untouched")

    if text == original:
        return
    if action == "remove" and not path.exists():
        return
    write_atomically(path, text)


if __name__ == "__main__":
    main(sys.argv)
