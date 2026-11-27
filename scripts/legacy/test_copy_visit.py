#!/usr/bin/env python3
"""Synthetic integration tests; never opens the real rules.json or its paths."""
import csv
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime
from unittest import mock

sys.dont_write_bytecode = True

SCRIPT = Path(__file__).resolve().with_name("copy_visit.py")
SPEC = importlib.util.spec_from_file_location("copy_visit", SCRIPT)
COPIER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(COPIER)


class CopyVisitsTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="grainatlas-copy-case-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.src = self.root / "source"
        self.v24, self.v25 = self.root / "visit24", self.root / "visit25"
        self.rules = self.root / "fake_rules.json"
        self.put("first.bin", b"same bytes")
        self.put("pkg/tiles/a.jpg", b"same bytes")
        self.put("pkg/tiles/a.txt", "Date=04/05/2024\nSignalName=SE\nMagnification=150\n".encode("utf-16"))
        self.put("pkg/tiles/b.jpg", b"tile b")
        self.put("pkg/tiles/b.txt", b"Date=04/03/2024\nSignalName=BSE\nMagnification=200\n")
        self.put("pkg/overview.zag", b"<SignalType1>UVD</SignalType1>")
        self.put("pkg/no_sidecar.dat", b"package metadata")
        self.put("pkg/nested/private.dat", b"child package")
        self.put("pkg/tiles/nested/keep.dat", b"exclude only direct subfolders")
        self.put("pkg/Thumbs.db", b"skip")
        self.put("captures/SJ2-A_x.jpg", b"single A")
        self.put("captures/SJ2-A_x.txt", b"Date=05/06/2025\nSignalName=UVD\nMagnification=50\n")
        self.put("captures/KD1-2_zircon(x80).jpg", b"single KD")
        self.put("captures/sj2_x.jpg", b"case insensitive sample")
        self.put("captures/SJ20_x.jpg", b"longest sample")
        self.put("captures/CameraImage_nav.jpg", b"navigation")
        self.put("captures/CameraImage_nav.txt", b"Date=05/07/2025\nSignalName=SE\nMagnification=0\n")
        self.put("captures/.DS_Store", b"skip too")
        self.put("captures/later_duplicate.jpg", b"same bytes")
        self.put("old/replaced.jpg", b"older export")
        self.put("old/absent.jpg", b"no replacement exists")
        self.put("new/replaced.jpg", b"newer export")
        self.put("v25/shared.bin", b"same bytes")
        self.config = dict(
            source_root="source", visits={"2024": "visit24", "2025": "visit25"},
            forbidden_write_roots=["forbidden"], skip_names=["Thumbs.db", ".DS_Store"],
            known_samples=["SJ2", "SJ2-A", "SJ20", "KD1-2"],
            coverage_roots=["pkg", "captures", "old", "new", "v25"], rules=[
                dict(visit="2024", src="first.bin", dst="first.bin", kind="document"),
                dict(visit="2024", src="pkg/nested", dst="SEM/child", package=True),
                dict(visit="2024", src="pkg", dst="SEM/pkg", package=True,
                     exclude_dirs=["nested"], kind="tm4000_package", mount="mount A",
                     samples="SJ2;KD1-2", notes="rule note"),
                dict(visit="2024", src="captures", match="CameraImage_*", dst="nav", kind="sem_navigation_camera"),
                dict(visit="2024", src="captures", match="*", dst="captures", kind="sem_single_capture"),
                dict(visit="2024", src="old", match="*", dst=None,
                     superseded_by_dir="new", notes="old export"),
                dict(visit="2024", src="new", match="*", dst="printouts"),
                dict(visit="2025", src="v25", match="*", dst="images"),
            ])
        self.save()

    def put(self, relative, data):
        path = self.src / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        os.utime(path, (1609502400, 1609502400))
        return path

    def save(self):
        self.rules.write_text(json.dumps(self.config), encoding="utf-8")

    def run_script(self, *args, success=True):
        result = subprocess.run([sys.executable, str(SCRIPT), str(self.rules), *args],
                                cwd=self.root, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0 if success else 1, result.stdout + result.stderr)
        return result.stdout + result.stderr

    def rows(self, path):
        with path.open(encoding="utf-8", newline="") as stream:
            reader = csv.DictReader(stream)
            self.assertEqual(reader.fieldnames, COPIER.COLUMNS)
            return list(reader)

    def snapshot(self):
        return {str(p.relative_to(self.src)): (p.read_bytes(), p.stat().st_mtime_ns)
                for p in self.src.rglob("*") if p.is_file()}

    def test_dry_execute_and_idempotency(self):
        for path in self.src.rglob("*"):
            if path.is_file():
                path.chmod(0o444)
        original = self.snapshot()
        output = self.run_script()
        self.assertIn("WOULD COPY", output)
        self.assertIn("skip_names", output)
        self.assertIn("claimed by an earlier rule", output)
        self.assertFalse(self.v24.exists())
        self.assertFalse(self.v25.exists())
        planned = self.rows(self.root / "plan_2024.csv")
        self.assertEqual(planned, sorted(planned, key=lambda r: (
            not bool(r["destination_path"]), r["destination_path"], r["source_path"])))
        by_source = {Path(r["source_path"]).relative_to(self.src).as_posix(): r for r in planned}
        a = by_source["pkg/tiles/a.jpg"]
        self.assertEqual(a["date"], "2024-04-05")
        self.assertEqual(a["kind"], "tm4000_package")
        self.assertEqual(a["mount"], "mount A")
        self.assertEqual(a["samples"], "SJ2;KD1-2")
        self.assertTrue(a["notes"].startswith("rule note; "))
        self.assertIn("SE 150x", a["notes"])
        self.assertIn("same bytes as " + str(self.src / "first.bin"), a["notes"])
        self.assertIn("CL (SignalType1=UVD)", a["notes"])
        self.assertEqual(by_source["pkg/no_sidecar.dat"]["date"], "2024-04-03")
        self.assertEqual(by_source["pkg/tiles/b.txt"]["date"], "2024-04-03")
        self.assertEqual(by_source["pkg/nested/private.dat"]["destination_path"], "SEM/child/private.dat")
        self.assertIn("pkg/tiles/nested/keep.dat", by_source)
        self.assertEqual(by_source["captures/SJ2-A_x.jpg"]["samples"], "SJ2-A")
        self.assertEqual(by_source["captures/KD1-2_zircon(x80).jpg"]["samples"], "KD1-2")
        self.assertEqual(by_source["captures/sj2_x.jpg"]["samples"], "SJ2")
        self.assertEqual(by_source["captures/SJ20_x.jpg"]["samples"], "SJ20")
        self.assertEqual(by_source["captures/CameraImage_nav.jpg"]["destination_path"], "nav/CameraImage_nav.jpg")
        self.assertNotIn("SE 0x", by_source["captures/CameraImage_nav.jpg"]["notes"])
        self.assertIn("date from file time", by_source["first.bin"]["notes"])
        self.assertEqual(by_source["captures/later_duplicate.jpg"]["duplicate_of"], str(self.src / "first.bin"))
        self.assertEqual(by_source["captures/later_duplicate.jpg"]["destination_path"], "")
        self.assertIn("superseded by new/replaced.jpg", by_source["old/replaced.jpg"]["notes"])
        self.assertNotIn("superseded by", by_source["old/absent.jpg"]["notes"])
        self.assertEqual(self.rows(self.root / "plan_2025.csv")[0]["duplicate_of"], "")
        output = self.run_script("--execute")
        self.assertIn("Verification 2024:", output)
        self.assertIn("mismatched=0, missing=0", output)
        for visit in (self.v24, self.v25):
            for row in self.rows(visit / "MANIFEST.csv"):
                if row["destination_path"]:
                    dest = visit / row["destination_path"]
                    self.assertEqual(hashlib.sha256(dest.read_bytes()).hexdigest(), row["sha256"])
        self.assertFalse((self.v24 / "captures/later_duplicate.jpg").exists())
        times = {p: p.stat().st_mtime_ns for p in self.v24.rglob("*") if p.is_file() and p.name != "MANIFEST.csv"}
        manifest_before = (self.v24 / "MANIFEST.csv").read_bytes()
        output = self.run_script("--execute")
        self.assertEqual((self.v24 / "MANIFEST.csv").read_bytes(), manifest_before)
        self.assertIn("ALREADY PRESENT", output)
        for p, mtime in times.items():
            self.assertEqual(p.stat().st_mtime_ns, mtime)
        self.assertEqual(self.snapshot(), original)
        self.assertIn("WOULD SKIP", self.run_script())
        self.assertFalse(list(self.root.rglob(".copy-*")))
        self.assertFalse(list(self.root.rglob(".manifest-*")))

    def test_empty_sidecar_values_do_not_swallow_next_line(self):
        path = self.put("empty.txt", b"AreaName=\nDate=04/05/2024\nSignalName=\nMagnification=150\n")
        fields, date = COPIER.sidecar_info(path)
        self.assertEqual(fields["areaname"], "")
        self.assertEqual(fields["signalname"], "")
        self.assertEqual(fields["magnification"], "150")
        self.assertEqual(date, "2024-04-05")

    def test_generic_visit_key(self):
        self.config["visits"]["2030_Lab"] = self.config["visits"].pop("2025")
        for rule in self.config["rules"]:
            if rule["visit"] == "2025":
                rule["visit"] = "2030_Lab"
        self.save()
        self.run_script("--visit", "2030_Lab")
        self.assertTrue((self.root / "plan_2030_Lab.csv").exists())

    def test_coverage_checks_all_visits_before_writes(self):
        self.put("v25/unclaimed/sub/file.bin", b"missed")
        output = self.run_script("--execute", "--visit", "2024", success=False)
        self.assertIn("unclaimed source files", output)
        self.assertIn("sub/file.bin", output)
        self.assertFalse(self.v24.exists())

    def test_collision_before_writes(self):
        self.config["rules"].insert(1, dict(visit="2024", src="new/replaced.jpg", dst="first.bin"))
        self.save()
        self.assertIn("destination collision", self.run_script("--execute", success=False))
        self.assertFalse(self.v24.exists())

    def test_cross_visit_collision_before_writes(self):
        self.config["visits"]["2025"] = "visit24"
        self.save()
        self.assertIn("collision", self.run_script("--execute", success=False))
        self.assertFalse(self.v24.exists())

    def test_reserved_manifest_and_ancestor_collision(self):
        for target in ("MANIFEST.csv", "first.bin/child"):
            self.config["rules"][2]["dst"] = target
            # Package target MANIFEST.csv creates a file/directory conflict.
            self.save()
            self.assertIn("collision", self.run_script("--execute", success=False))
            self.assertFalse(self.v24.exists())

    def test_protected_destinations_and_symlinks(self):
        forbidden = self.root / "forbidden"
        forbidden.mkdir()
        for target in ("source/outputs", "forbidden", "alias"):
            if target == "alias":
                (self.root / "alias").symlink_to(forbidden, target_is_directory=True)
            self.config["visits"]["2024"] = target
            self.save()
            self.assertIn("protected path", self.run_script("--execute", success=False))
            self.assertFalse((self.src / "outputs").exists())
            self.assertEqual(list(forbidden.iterdir()), [])
        self.config["visits"]["2024"] = "visit24"
        self.save()
        self.v24.mkdir()
        (self.v24 / "SEM").symlink_to(self.src / "pkg", target_is_directory=True)
        self.assertIn("protected path", self.run_script("--execute", success=False))
        self.assertFalse((self.v24 / "first.bin").exists())

    def test_existing_conflict_continues_and_verifies(self):
        self.v24.mkdir()
        conflict = self.v24 / "first.bin"
        conflict.write_bytes(b"do not overwrite")
        output = self.run_script("--execute", "--visit", "2024", success=False)
        self.assertEqual(conflict.read_bytes(), b"do not overwrite")
        self.assertTrue((self.v24 / "SEM/pkg/tiles/a.jpg").is_file())
        row = next(r for r in self.rows(self.v24 / "MANIFEST.csv") if r["destination_path"] == "first.bin")
        self.assertIn("ERROR:", row["notes"])
        self.assertIn("mismatched=1, missing=0", output)
        self.assertFalse(self.v25.exists())

    def test_manifest_only_and_visit_selection(self):
        self.run_script("--manifest-only", "--visit", "2024")
        self.assertFalse(self.v24.exists())
        self.assertFalse((self.root / "plan_2025.csv").exists())
        output = self.run_script("--execute", "--manifest-only", "--visit", "2024", success=False)
        self.assertTrue((self.v24 / "MANIFEST.csv").is_file())
        self.assertFalse((self.v24 / "first.bin").exists())
        self.assertIn("verified=0", output)
        self.run_script("--execute", "--visit", "2024")
        self.run_script("--execute", "--manifest-only", "--visit", "2024")
        self.assertFalse(self.v25.exists())

    def test_bad_copy_is_not_published(self):
        src = self.src / "first.bin"
        dest = self.v24 / "file.bin"
        row = {"source_path": str(src), "sha256": COPIER.digest(src)}
        with mock.patch.object(COPIER.shutil, "copy2", side_effect=lambda a, b: Path(b).write_bytes(b"corrupt")):
            with self.assertRaisesRegex(ValueError, "SHA-256"):
                COPIER.copy_file(row, dest, self.src, [])
        self.assertFalse(dest.exists())
        self.assertFalse(list(self.v24.glob(".copy-*")))

    def test_exif_and_without_pillow(self):
        path = self.src / "first.bin"
        with mock.patch.object(COPIER, "Image", None):
            date, note = COPIER.file_date(path, "", "")
            self.assertEqual(date, datetime.fromtimestamp(path.stat().st_mtime).date().isoformat())
            self.assertEqual(note, "date from file time")
        self.assertIsNotNone(COPIER.Image, "EXIF coverage requires Pillow: python3 -m pip install Pillow")
        image_path = self.src / "photo.jpg"
        exif = COPIER.Image.Exif()
        exif[36867] = "2023:06:07 08:09:10"
        COPIER.Image.new("RGB", (2, 2)).save(image_path, exif=exif)
        self.assertEqual(COPIER.file_date(image_path, "", ""), ("2023-06-07", ""))
        self.assertEqual(COPIER.file_date(image_path, "2024-01-01", "2022-01-01"), ("2024-01-01", ""))
        exif[36867] = "2019:06:07 08:09:10"
        COPIER.Image.new("RGB", (2, 2)).save(image_path, exif=exif)
        self.assertEqual(COPIER.file_date(image_path, "", "")[1], "date from file time")


if __name__ == "__main__":
    unittest.main(verbosity=2)
