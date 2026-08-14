#!/usr/bin/env python3
"""Build a platform wheel from a generated Vifu Python SDK directory."""

from __future__ import annotations

import argparse
import base64
import csv
import hashlib
import io
import os
import re
import sys
import tomllib
import zipfile
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
PYPROJECT_PATH = REPOSITORY_ROOT / "sdk" / "python" / "pyproject.toml"
README_PATH = REPOSITORY_ROOT / "sdk" / "python" / "README.md"
LICENSE_PATHS = (REPOSITORY_ROOT / "LICENSE", REPOSITORY_ROOT / "NOTICE")


def _wheel_name(value: str) -> str:
    return re.sub(r"[-_.]+", "_", value)


def _metadata(project: dict[str, object]) -> bytes:
    lines = [
        "Metadata-Version: 2.4",
        f"Name: {project['name']}",
        f"Version: {project['version']}",
        f"Summary: {project['description']}",
        f"Requires-Python: {project['requires-python']}",
        f"License-Expression: {project['license']}",
        "Description-Content-Type: text/markdown",
    ]
    authors = [author["name"] for author in project.get("authors", [])]
    if authors:
        lines.append(f"Author: {', '.join(authors)}")
    keywords = project.get("keywords", [])
    if keywords:
        lines.append(f"Keywords: {','.join(keywords)}")
    for classifier in project.get("classifiers", []):
        lines.append(f"Classifier: {classifier}")
    for extra, requirements in project.get("optional-dependencies", {}).items():
        lines.append(f"Provides-Extra: {extra}")
        for requirement in requirements:
            lines.append(f'Requires-Dist: {requirement}; extra == "{extra}"')
    for label, url in project.get("urls", {}).items():
        lines.append(f"Project-URL: {label}, {url}")
    lines.extend(
        [
            "License-File: LICENSE",
            "License-File: NOTICE",
            "",
            README_PATH.read_text(encoding="utf-8"),
        ]
    )
    return "\n".join(lines).encode()


def _digest(data: bytes) -> str:
    value = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=")
    return f"sha256={value.decode()}"


def _zip_info(path: str, *, executable: bool = False) -> zipfile.ZipInfo:
    info = zipfile.ZipInfo(path, date_time=(1980, 1, 1, 0, 0, 0))
    info.compress_type = zipfile.ZIP_DEFLATED
    info.external_attr = (0o100755 if executable else 0o100644) << 16
    return info


def build_wheel(package_dir: Path, output_dir: Path, platform_tag: str) -> Path:
    configuration = tomllib.loads(PYPROJECT_PATH.read_text(encoding="utf-8"))
    project = configuration["project"]
    distribution = _wheel_name(project["name"])
    version = project["version"]
    dist_info = f"{distribution}-{version}.dist-info"
    wheel_tag = f"py3-none-{platform_tag}"
    output_dir.mkdir(parents=True, exist_ok=True)
    wheel_path = output_dir / f"{distribution}-{version}-{wheel_tag}.whl"

    if not package_dir.is_dir():
        raise FileNotFoundError(f"Python package directory does not exist: {package_dir}")
    version_source = package_dir / "_version.py"
    version_match = re.search(
        r'^__version__\s*=\s*["\']([^"\']+)["\']$',
        version_source.read_text(encoding="utf-8"),
        flags=re.MULTILINE,
    )
    if version_match is None or version_match.group(1) != version:
        raise RuntimeError("The Python package version does not match pyproject.toml")
    native_libraries = [
        path
        for pattern in ("*.dylib", "*.so", "*.dll")
        for path in package_dir.glob(pattern)
    ]
    if len(native_libraries) != 1:
        raise RuntimeError("The Python wheel requires exactly one Vifu native library")
    server_candidates = (
        package_dir / "_bin" / "vifu",
        package_dir / "_bin" / "vifu.exe",
    )
    server_binaries = [path for path in server_candidates if path.is_file()]
    if len(server_binaries) != 1:
        raise RuntimeError("The Python wheel requires exactly one Vifu Server binary")

    files: dict[str, bytes] = {}
    for source in sorted(package_dir.rglob("*")):
        if source.is_file() and "__pycache__" not in source.parts and source.suffix != ".pyc":
            files[f"vifu/{source.relative_to(package_dir).as_posix()}"] = source.read_bytes()

    files[f"{dist_info}/METADATA"] = _metadata(project)
    files[f"{dist_info}/WHEEL"] = (
        "Wheel-Version: 1.0\n"
        "Generator: vifu-wheel-builder\n"
        "Root-Is-Purelib: false\n"
        f"Tag: {wheel_tag}\n"
    ).encode()
    for license_path in LICENSE_PATHS:
        files[f"{dist_info}/licenses/{license_path.name}"] = license_path.read_bytes()

    record_path = f"{dist_info}/RECORD"
    record_buffer = io.StringIO(newline="")
    writer = csv.writer(record_buffer, lineterminator="\n")
    for path, data in sorted(files.items()):
        writer.writerow((path, _digest(data), len(data)))
    writer.writerow((record_path, "", ""))
    files[record_path] = record_buffer.getvalue().encode()

    with zipfile.ZipFile(wheel_path, "w") as archive:
        for path, data in sorted(files.items()):
            executable = path in {"vifu/_bin/vifu", "vifu/_bin/vifu.exe"}
            archive.writestr(_zip_info(path, executable=executable), data)
    return wheel_path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--package-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--platform-tag", required=True)
    arguments = parser.parse_args()

    wheel_path = build_wheel(
        arguments.package_dir.resolve(),
        arguments.output_dir.resolve(),
        arguments.platform_tag,
    )
    print(wheel_path)
    return 0


if __name__ == "__main__":
    sys.exit(main())
