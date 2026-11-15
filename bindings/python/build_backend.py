"""PEP 517 backend: cargo builds the abi3 extension, which goes into a PEP 427 wheel."""

import base64
import hashlib
import json
import os
import re
import subprocess
import sys
import sysconfig
import tomllib
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
LIBRARY = {"darwin": ".dylib", "win32": ".dll"}.get(sys.platform, ".so")
MODULE = "starlightpdf.pyd" if sys.platform == "win32" else "starlightpdf.abi3.so"
TAG = "cp311-abi3-" + re.sub(r"[-.]", "_", sysconfig.get_platform())


def _library():
    env = {**os.environ, "PYO3_PYTHON": sys.executable, "PYO3_BUILD_EXTENSION_MODULE": "1"}
    link = []
    if sys.platform == "darwin":
        link = ["-C", "link-arg=-undefined", "-C", "link-arg=dynamic_lookup"]
    run = subprocess.run(
        ["cargo", "rustc", "--locked", "--release", "--lib", "--message-format=json-render-diagnostics", "--", *link],
        cwd=HERE,
        env=env,
        stdout=subprocess.PIPE,
        encoding="utf-8",
        check=True,
    )
    for line in run.stdout.splitlines():
        message = json.loads(line)
        if message["reason"] == "compiler-artifact" and "cdylib" in message["target"]["crate_types"]:
            return Path(next(f for f in message["filenames"] if f.endswith(LIBRARY)))
    raise RuntimeError("cargo built no cdylib")


def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    with open(HERE / "pyproject.toml", "rb") as f:
        requires = tomllib.load(f)["project"]["requires-python"]
    with open(HERE.parents[1] / "Cargo.toml", "rb") as f:
        version = tomllib.load(f)["workspace"]["package"]["version"]
    info = f"starlightpdf-{version}.dist-info"
    files = {
        MODULE: _library().read_bytes(),
        f"{info}/WHEEL": f"Wheel-Version: 1.0\nRoot-Is-Purelib: false\nTag: {TAG}\n".encode(),
        f"{info}/METADATA": (
            f"Metadata-Version: 2.1\nName: starlightpdf\nVersion: {version}\nRequires-Python: {requires}\n".encode()
        ),
    }
    record = "".join(
        f"{path},sha256={base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b'=').decode()},{len(data)}\n"
        for path, data in files.items()
    )
    files[f"{info}/RECORD"] = (record + f"{info}/RECORD,,\n").encode()
    name = f"starlightpdf-{version}-{TAG}.whl"
    with zipfile.ZipFile(Path(wheel_directory) / name, "w", zipfile.ZIP_DEFLATED) as wheel:
        for path, data in files.items():
            wheel.writestr(path, data)
    return name
