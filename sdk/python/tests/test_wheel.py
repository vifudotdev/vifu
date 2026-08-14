from __future__ import annotations

import csv
import importlib.util
import tempfile
import unittest
import zipfile
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
BUILDER_PATH = REPOSITORY_ROOT / "scripts" / "build-python-wheel.py"
SPEC = importlib.util.spec_from_file_location("vifu_wheel_builder", BUILDER_PATH)
assert SPEC is not None and SPEC.loader is not None
BUILDER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILDER)


class VifuWheelTests(unittest.TestCase):
    def test_wheel_contains_native_library_and_valid_record(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = root / "package"
            package.mkdir()
            (package / "__init__.py").write_text("VALUE = 1\n", encoding="utf-8")
            (package / "_version.py").write_text('__version__ = "0.1.1"\n', encoding="utf-8")
            (package / "libvifu_mobile_ffi.so").write_bytes(b"native-fixture")

            wheel = BUILDER.build_wheel(package, root / "dist", "linux_x86_64")

            self.assertEqual(
                wheel.name,
                "vifu-0.1.1-py3-none-linux_x86_64.whl",
            )
            with zipfile.ZipFile(wheel) as archive:
                names = set(archive.namelist())
                self.assertIn("vifu/libvifu_mobile_ffi.so", names)
                metadata = archive.read("vifu-0.1.1.dist-info/METADATA").decode()
                self.assertIn("Name: vifu\n", metadata)
                self.assertEqual(metadata.count("Keywords:"), 1)
                record = archive.read("vifu-0.1.1.dist-info/RECORD").decode()
                rows = list(csv.reader(record.splitlines()))
                self.assertEqual(rows[-1], ["vifu-0.1.1.dist-info/RECORD", "", ""])
                self.assertEqual({row[0] for row in rows}, names)

    def test_wheel_requires_one_native_library(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = root / "package"
            package.mkdir()
            (package / "__init__.py").write_text("", encoding="utf-8")
            (package / "_version.py").write_text('__version__ = "0.1.1"\n', encoding="utf-8")

            with self.assertRaisesRegex(RuntimeError, "exactly one"):
                BUILDER.build_wheel(package, root / "dist", "linux_x86_64")


if __name__ == "__main__":
    unittest.main()
