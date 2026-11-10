#!/usr/bin/env python3
"""Plan or copy legacy imaging. Relative root paths are based on the rules file.

--manifest-only with --execute writes manifests and verifies existing files,
without copying data. Without --execute, all modes only write plan_<visit>.csv.
Directory symlinks in source trees are rejected rather than silently omitted.
"""
import argparse
import csv
import fnmatch
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sys
import tempfile
from datetime import datetime
from functools import lru_cache

try:
    from PIL import Image
except ImportError:
    Image = None

COLUMNS = ["source_path", "destination_path", "sha256", "kind", "mount",
           "samples", "date", "notes", "duplicate_of"]
IMAGES = {".jpg", ".jpeg", ".tif", ".tiff", ".bmp", ".png", ".gif", ".webp"}


def digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def inside(path, root):
    return path == root or root in path.parents


def relative_path(root, value):
    p = Path(value)
    if p.is_absolute() or ".." in p.parts:
        raise ValueError(f"expected a relative path without '..': {value}")
    return root / p


def read_text(path):
    data = path.read_bytes()
    encoding = "utf-16" if data.startswith((b"\xff\xfe", b"\xfe\xff")) else "utf-8-sig"
    return data.decode(encoding, errors="replace")


@lru_cache(maxsize=None)
def sidecar_info(path):
    if not path.is_file():
        return {}, ""
    fields = dict(re.findall(r"^[ \t]*([^=\r\n]+?)[ \t]*=[ \t]*([^\r\n]*)",
                             read_text(path), flags=re.MULTILINE))
    fields = {key.lower(): value.strip() for key, value in fields.items()}
    try:
        date = datetime.strptime(fields.get("date", ""), "%m/%d/%Y").date().isoformat()
    except ValueError:
        date = ""
    return fields, date


def own_sidecar(path):
    if path.suffix.lower() == ".txt":
        return path
    sidecar = path.with_suffix(".txt")
    if not sidecar.is_file():
        sidecar = path.with_suffix(".TXT")
    return sidecar


def file_date(path, side_date, package_date):
    if side_date or package_date:
        return side_date or package_date, ""
    if Image is not None:
        try:
            with Image.open(path) as im:
                exif = im.getexif()
                value = exif.get(36867)  # DateTimeOriginal, sometimes in the Exif sub-IFD.
                if not value and hasattr(exif, "get_ifd"):
                    value = exif.get_ifd(34665).get(36867)
                if isinstance(value, bytes):
                    value = value.decode("ascii")
                date = datetime.strptime(str(value).strip("\x00"), "%Y:%m:%d %H:%M:%S")
                if date.year >= 2020:
                    return date.date().isoformat(), ""
        except (OSError, ValueError, TypeError, KeyError, SyntaxError):
            pass
    return datetime.fromtimestamp(path.stat().st_mtime).date().isoformat(), "date from file time"


def add_note(row, note):
    if note:
        row["notes"] = "; ".join(filter(None, [row["notes"], note]))


def tree_files(root, skip_names, exclude_dirs=(), report=False):
    if not root.is_dir():
        raise ValueError(f"missing source directory: {root}")
    found = []
    def fail(error):
        raise error

    for current, dirs, names in os.walk(root, onerror=fail):
        current = Path(current)
        for name in list(dirs):
            if current == root and name in exclude_dirs:
                dirs.remove(name)
                if report:
                    print(f"SKIP {current / name}: excluded direct subfolder")
            elif (current / name).is_symlink():
                raise ValueError(f"source directory symlink is unsupported: {current / name}")
        dirs.sort()
        for name in sorted(names):
            path = current / name
            if name in skip_names:
                if report:
                    print(f"SKIP {path}: skip_names")
            elif not path.is_file():
                raise ValueError(f"not a regular source file: {path}")
            else:
                found.append(path)
    return sorted(found)


def plan(config, base):
    source = (base / config["source_root"]).resolve()
    visits = {str(v): (base / path).absolute() for v, path in config["visits"].items()}
    skip = set(config.get("skip_names", []))
    claimed, groups = set(), []
    for rule in config["rules"]:
        visit = str(rule["visit"])
        if visit not in visits:
            raise ValueError(f"unknown visit: {visit}")
        src = relative_path(source, rule["src"])
        if rule.get("package"):
            files = tree_files(src, skip, rule.get("exclude_dirs", []), report=True)
        elif "match" in rule:
            if not src.is_dir():
                raise ValueError(f"missing source directory: {src}")
            files = sorted(p for p in src.iterdir()
                           if p.is_file() and fnmatch.fnmatchcase(p.name, rule["match"]))
        else:
            if not src.is_file():
                raise ValueError(f"missing source file: {src}")
            files = [src]
        winners = []
        for path in files:
            if not inside(path.resolve(), source):
                raise ValueError(f"source file resolves outside source_root: {path}")
            if path.name in skip:
                print(f"SKIP {path}: skip_names")
            elif path in claimed:
                print(f"SKIP {path}: claimed by an earlier rule")
            else:
                claimed.add(path)
                winners.append(path)
        groups.append((rule, src, files, winners))

    uncovered = set()
    for value in config.get("coverage_roots", []):
        uncovered.update(set(tree_files(relative_path(source, value), skip)) - claimed)
    if uncovered:
        raise ValueError("unclaimed source files:\n" + "\n".join(map(str, sorted(uncovered))))

    rows = {visit: [] for visit in visits}
    hashes = {visit: {} for visit in visits}
    samples = sorted(config.get("known_samples", []), key=len, reverse=True)
    for rule, src, files, winners in groups:
        visit = str(rule["visit"])
        package = rule.get("package", False)
        dates = [sidecar_info(p)[1] for p in files if package and p.suffix.lower() == ".txt"]
        package_date = min(filter(None, dates), default="")
        cl = package and any(p.suffix.lower() == ".zag" and
                             "SignalType1>UVD<" in read_text(p) for p in files)
        for path in winners:
            target = rule["dst"]
            if target is not None:
                target = relative_path(Path(), target)
                if package:
                    target /= path.relative_to(src)
                elif "match" in rule:
                    target /= path.name
                target = target.as_posix()
            row = dict.fromkeys(COLUMNS, "")
            row.update(source_path=str(path), destination_path=target or "",
                       sha256=digest(path), kind=rule.get("kind", ""),
                       mount=rule.get("mount", ""), samples=rule.get("samples", ""),
                       notes=rule.get("notes", ""))
            if "match" in rule and not row["samples"]:
                token = re.split(r"[_ (]", path.stem)[0].casefold()
                row["samples"] = next((s for s in samples if s.casefold() == token), "")
            fields, side_date = sidecar_info(own_sidecar(path))
            row["date"], note = file_date(path, side_date, package_date)
            add_note(row, note)
            if path.suffix.lower() in IMAGES and fields.get("signalname"):
                try:
                    mag = float(fields.get("magnification", "0"))
                    if mag > 0:
                        add_note(row, f"{fields['signalname']} {mag:g}x")
                except ValueError:
                    pass
            if cl:
                add_note(row, "CL (SignalType1=UVD)")
            if target is None and rule.get("superseded_by_dir"):
                replacement = relative_path(source, rule["superseded_by_dir"]) / path.name
                if replacement.is_file():
                    add_note(row, f"superseded by {rule['superseded_by_dir']}/{path.name}")
            first = hashes[visit].get(row["sha256"])
            if target is not None:
                if first and not package:
                    row.update(destination_path="", duplicate_of=first)
                else:
                    if first:
                        add_note(row, f"same bytes as {first}")
                    hashes[visit].setdefault(row["sha256"], str(path))
            rows[visit].append(row)
    for visit_rows in rows.values():
        visit_rows.sort(key=lambda row: (not bool(row["destination_path"]),
                                        row["destination_path"], row["source_path"]))
    return source, visits, rows


def safe_write(path, source, forbidden):
    resolved = path.resolve()
    if any(inside(resolved, root) for root in [source] + forbidden):
        raise ValueError(f"refusing write to protected path: {path} -> {resolved}")
    return resolved


def preflight(source, visits, rows, selected, forbidden, execute):
    targets = {}
    for visit in selected:
        root = visits[visit]
        safe_write(root, source, forbidden)
        manifest = safe_write(root / "MANIFEST.csv", source, forbidden)
        if manifest in targets or (manifest.exists() and not manifest.is_file()):
            raise ValueError(f"manifest destination collision: {manifest}")
        targets[manifest] = f"visit {visit} MANIFEST.csv (reserved)"
        for row in rows[visit]:
            if not row["destination_path"]:
                continue
            dest = root / row["destination_path"]
            resolved = safe_write(dest, source, forbidden)
            if not inside(resolved, root.resolve()):
                raise ValueError(f"destination escapes visit folder: {dest}")
            if resolved in targets:
                raise ValueError(f"destination collision: {dest}: {targets[resolved]} and {row['source_path']}")
            targets[resolved] = row["source_path"]
        if not execute:
            safe_write(Path.cwd() / f"plan_{visit}.csv", source, forbidden)
    for target in targets:
        if any(parent in targets for parent in target.parents):
            raise ValueError(f"file/directory destination collision: {target}")
        if any(parent.exists() and not parent.is_dir() for parent in target.parents):
            raise ValueError(f"destination parent is not a directory: {target}")


def write_manifest(path, rows, source, forbidden):
    safe_write(path, source, forbidden)
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", newline="",
                                         dir=path.parent, prefix=".manifest-", delete=False) as stream:
            temp = Path(stream.name)
            writer = csv.DictWriter(stream, fieldnames=COLUMNS)
            writer.writeheader()
            writer.writerows(rows)
        safe_write(path, source, forbidden)
        os.replace(temp, path)
    finally:
        if temp is not None:
            temp.unlink(missing_ok=True)


def existing_status(dest, expected_hash):
    if dest.exists():
        if dest.is_file() and digest(dest) == expected_hash:
            return "already present"
        raise ValueError("destination exists with different bytes or is not a file")
    if dest.is_symlink():
        raise ValueError("destination is a dangling symlink")
    return ""


def copy_file(row, dest, source, forbidden):
    safe_write(dest, source, forbidden)
    status = existing_status(dest, row["sha256"])
    if status:
        return status
    dest.parent.mkdir(parents=True, exist_ok=True)
    temp = None
    try:
        with tempfile.NamedTemporaryFile(dir=dest.parent, prefix=".copy-", delete=False) as stream:
            temp = Path(stream.name)
        shutil.copy2(row["source_path"], temp)
        if digest(temp) != row["sha256"]:
            raise ValueError("temporary copy failed SHA-256 verification")
        safe_write(dest, source, forbidden)
        status = existing_status(dest, row["sha256"])
        if status:
            return status
        os.rename(temp, dest)
        return "copied"
    finally:
        if temp is not None:
            temp.unlink(missing_ok=True)


def verify(root, rows):
    counts = dict(files=0, bytes=0, verified=0, mismatched=0, missing=0)
    for row in rows:
        if not row["destination_path"]:
            continue
        counts["files"] += 1
        dest = root / row["destination_path"]
        try:
            counts["bytes"] += dest.stat().st_size
            counts["verified" if digest(dest) == row["sha256"] else "mismatched"] += 1
        except FileNotFoundError:
            counts["missing"] += 1
        except OSError as error:
            counts["mismatched"] += 1
            print(f"ERROR verifying {dest}: {error}", file=sys.stderr)
    return counts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("rules", type=Path)
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--visit", help="Visit key from the rules file")
    parser.add_argument("--manifest-only", action="store_true")
    args = parser.parse_args()
    try:
        config = json.loads(args.rules.read_text(encoding="utf-8"))
        base = args.rules.absolute().parent
        source, visits, rows = plan(config, base)
        selected = [args.visit] if args.visit else list(visits)
        if any(v not in visits for v in selected):
            raise ValueError(f"visit not configured: {args.visit}")
        forbidden = [(base / p).resolve() for p in config.get("forbidden_write_roots", [])]
        preflight(source, visits, rows, selected, forbidden, args.execute)
        failed = False
        for visit in selected:
            counts = dict(copied=0, already_present=0, skipped=0, errors=0, planned=0)
            for row in rows[visit]:
                if not row["destination_path"]:
                    counts["skipped"] += 1
                    reason = (f"duplicate of {row['duplicate_of']}" if row["duplicate_of"]
                              else "dst is null")
                    print(f"SKIP {row['source_path']}: {reason}")
                    continue
                dest = visits[visit] / row["destination_path"]
                try:
                    if not args.execute:
                        status = existing_status(dest, row["sha256"])
                        if status:
                            counts["already_present"] += 1
                            print(f"WOULD SKIP {dest}: {status}")
                        else:
                            counts["planned"] += 1
                            print(f"WOULD {'CHECK' if args.manifest_only else 'COPY'} {row['source_path']} -> {dest}")
                        continue
                    elif args.manifest_only:
                        status = existing_status(dest, row["sha256"])
                        if not status:
                            raise ValueError("manifest-only: destination missing")
                    else:
                        status = copy_file(row, dest, source, forbidden)
                    counts[status.replace(" ", "_")] += 1
                    print(f"{status.upper()} {dest}")
                except (OSError, ValueError) as error:
                    counts["errors"] += 1
                    add_note(row, f"ERROR: {error}")
                    print(f"ERROR {dest}: {error}", file=sys.stderr)
            manifest = (visits[visit] / "MANIFEST.csv" if args.execute
                        else Path.cwd() / f"plan_{visit}.csv")
            write_manifest(manifest, rows[visit], source, forbidden)
            print(f"Visit {visit}: " + ", ".join(f"{k}={v}" for k, v in counts.items()))
            print(f"Manifest: {manifest}")
            failed |= bool(counts["errors"])
            if args.execute:
                result = verify(visits[visit], rows[visit])
                print(f"Verification {visit}: " + ", ".join(f"{k}={v}" for k, v in result.items()))
                failed |= bool(result["missing"] or result["mismatched"])
        return int(failed)
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
