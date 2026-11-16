"""Copy chosen areas of a TM4000 Zigzag package with neutral names and no session metadata.

For docs screenshots only (docs/screenshots.md). Write into an ignored folder under target/;
the copy holds real images and must never be committed.

usage: python3 -I scripts/neutralise_package.py SRC_PACKAGE OUT_PARENT NEW_NAME OLD_FOLDER=NEW_AREA [...]
Writes OUT_PARENT/NEW_NAME/ with area folders NNN_<NEW_AREA>. Pixels are copied unchanged;
the .zag keeps only the chosen areas, and names, comments, paths, users, serials, dates and
times in the sidecars are blanked or fixed.
"""
import re
import shutil
import sys
from pathlib import Path

src, out_parent, new_name = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
mapping = {}  # old folder -> (new folder, new area name)
for i, arg in enumerate(sys.argv[4:], 1):
    old, area = arg.split("=")
    mapping[old] = (f"{i:03d}_{area}", area)

if "target" not in out_parent.resolve().parts:
    sys.exit("OUT_PARENT must be inside an ignored target/ folder")
old_pkg = next(src.glob("*.zag")).stem
out = out_parent / new_name
if out.exists():
    shutil.rmtree(out)
out.mkdir(parents=True)

BLANK = {"Directory", "SerialNumber", "DataNumber", "Comment", "SampleName", "UserName", "Media",
         "KeyWord1", "KeyWord2", "SDMVersion"}
FIXED = {"Date": "01/01/2026", "Time": "00:00:00"}


def rename_text(text: str, old: str, new: str) -> str:
    return text.replace(old, new)


# .zag
zag = (src / f"{old_pkg}.zag").read_text(encoding="utf-8-sig")
zag = re.sub(r"\s*<!--.*?-->", "", zag, flags=re.S)
defs = re.findall(r"\s*<AreaDefinition Count=\"\d+\">.*?</AreaDefinition>", zag, flags=re.S)
kept = []
for d in defs:
    folder = re.search(r"<FolderName>(.*?)</FolderName>", d).group(1)
    if folder not in mapping:
        continue
    nf, area = mapping[folder]
    n = len(kept) + 1
    d = re.sub(r"Count=\"\d+\"", f'Count="{n}"', d, count=1)
    d = re.sub(r"<AreaNo>.*?</AreaNo>", f"<AreaNo>Area{n}</AreaNo>", d)
    d = re.sub(r"<AreaName>.*?</AreaName>", f"<AreaName>{area}</AreaName>", d)
    d = re.sub(r"<FolderName>.*?</FolderName>", f"<FolderName>{nf}</FolderName>", d)
    for tag in ("Sample", "Keyword1", "Keyword2", "Comment"):
        d = re.sub(rf"<{tag}>.*?</{tag}>", f"<{tag}></{tag}>", d)
    kept.append(d)
head = zag[: zag.index(defs[0])]
tail = zag[zag.index(defs[-1]) + len(defs[-1]):]
head = re.sub(r'TotalSelectArea="\d+"', f'TotalSelectArea="{len(kept)}"', head)
(out / f"{new_name}.zag").write_text("﻿" + head + "".join(kept) + tail, encoding="utf-8")

for nav in src.glob(f"{old_pkg}.bmp"):
    shutil.copyfile(nav, out / f"{new_name}.bmp")

for old, (nf, area) in mapping.items():
    d = out / nf
    d.mkdir()
    for f in sorted((src / old).iterdir()):
        name = f.name.replace(old, nf)
        if f.suffix.lower() == ".jpg":
            shutil.copyfile(f, d / name)
            continue
        if f.suffix.lower() not in (".txt", ".ini", ".xml"):
            continue
        raw = f.read_bytes()
        utf16 = raw.startswith(b"\xff\xfe")
        text = raw.decode("utf-16" if utf16 else "utf-8-sig")
        text = text.replace(old, nf)
        if f.suffix.lower() == ".txt":
            lines = []
            for line in text.splitlines():
                k, sep, v = line.partition("=")
                if sep and k in BLANK:
                    line = f"{k}="
                elif sep and k in FIXED:
                    line = f"{k}={FIXED[k]}"
                lines.append(line)
            text = "\r\n".join(lines) + "\r\n"
        data = ("﻿" + text).encode("utf-16-le") if utf16 else ("﻿" + text).encode("utf-8")
        (d / name).write_bytes(data)
print(out)
