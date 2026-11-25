#!/usr/bin/env python3
"""Start pinned Albatross native inference on a CUDA host over loopback."""

import argparse
import hashlib
import os
import platform
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request
from pathlib import Path


VERSION = "1.8.15"
SERVER = "rwkv_lighting_cuda"
VOCAB = "rwkv_vocab_v20230424.txt"
RELEASE = f"https://github.com/Alic-Li/rwkv_lightning_cuda/releases/download/v{VERSION}"
ARCHIVES = {
    ("linux", "12.9"): (986150311, "a1fc44038f8e1e3a477bc2af128fa2ff3a2ae1c351a3bc4c4b1a238e987cb6f6"),
    ("linux", "13.2"): (675462338, "20befa2aaa8241821bf779846c25616618e0faa16ea1b82c45df9244b379bde7"),
    ("windows", "12.9"): (612780108, "0b6f5dee7e2a9b18cb6e001286d059c02aac939e18f8b75e6d89320782fd11dd"),
    ("windows", "13.2"): (445082409, "09d48926d0eec5ca3c75127b933e682e02277e537102db791fbbf1efc874e799"),
}
MODEL_NAME = "rwkv7-g1l-1.5b-20261031-ctx34304.pth"
MODEL_URL = f"https://huggingface.co/BlinkDL/rwkv7-g1/resolve/11e3c50128e479898eee79c48831a95cf3e21d91/{MODEL_NAME}"
MODEL_BYTES = 3055444605
MODEL_SHA256 = "b0753acefe6c9ea0b4e2e9d60a3c2faa76d712a1d607db187301aae13dadff41"


def checked_download(url: str, destination: Path, size: int, sha256: str) -> None:
    print(f"Downloading and verifying {destination.name} ({size / 1e9:.2f} GB)", flush=True)
    request = urllib.request.Request(url, headers={"User-Agent": "RWKV-Vibe-Code"})
    digest = hashlib.sha256()
    received = 0
    with urllib.request.urlopen(request, timeout=60) as response, destination.open("xb") as output:
        while chunk := response.read(1024 * 1024):
            received += len(chunk)
            if received > size:
                raise ValueError(f"{destination.name} exceeds the pinned download size")
            digest.update(chunk)
            output.write(chunk)
    if received != size or digest.hexdigest() != sha256:
        raise ValueError(f"{destination.name} failed its size or SHA-256 check")


def unpack_release(archive: Path, destination: Path) -> None:
    # Every downloaded archive reaches this step only after pinned SHA-256 verification.
    if archive.suffix == ".zip":
        shutil.unpack_archive(archive, destination)
    else:
        with tarfile.open(archive, "r:gz") as source:
            if hasattr(tarfile, "data_filter"):
                source.extractall(destination, filter="data")
            else:
                source.extractall(destination)


def runtime_bundle(root: Path, system: str, executable: str, cuda: str, download: bool) -> Path:
    for folder in (root, root / "bundle"):
        if (folder / executable).is_file() and (folder / VOCAB).is_file():
            return folder
    if not download:
        raise ValueError(f"No {executable} and {VOCAB} found in {root} or its bundle directory")

    target = root / "bundle"
    if target.exists():
        raise ValueError(f"Incomplete existing runtime at {target}. Use a new download directory")
    root.mkdir(parents=True, exist_ok=True)
    extension = "zip" if system == "windows" else "tar.gz"
    filename = f"rwkv-lightning-v{VERSION}-{system}-x64-cuda{cuda}.{extension}"
    size, sha256 = ARCHIVES[system, cuda]
    with tempfile.TemporaryDirectory(dir=root, prefix=".albatross-") as directory:
        staging = Path(directory)
        archive = staging / filename
        checked_download(f"{RELEASE}/{filename}", archive, size, sha256)
        unpack_release(archive, staging)
        bundle = staging / "bundle"
        if not (bundle / executable).is_file() or not (bundle / VOCAB).is_file():
            raise ValueError("The archive does not have the expected Albatross bundle layout")
        bundle.rename(target)
    return target


def model_file(path: Path, download: bool) -> Path:
    if path.suffix not in {".pth", ".rwkvq"}:
        raise ValueError("Use a trusted RWKV-7 .pth or Albatross .rwkvq checkpoint")
    if path.is_file():
        return path
    if not download or path.name != MODEL_NAME:
        raise ValueError(f"Checkpoint not found: {path}. --download provides {MODEL_NAME}")
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=path.parent, prefix=".albatross-model-") as directory:
        staged = Path(directory) / path.name
        checked_download(MODEL_URL, staged, MODEL_BYTES, MODEL_SHA256)
        staged.replace(path)
    return path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime-dir", type=Path, required=True, help="Extracted bundle or its parent directory")
    parser.add_argument("--model", type=Path, default=Path.home() / "Models" / "rwkv" / MODEL_NAME,
                        help=f"Trusted .pth or .rwkvq checkpoint (default: ~/Models/rwkv/{MODEL_NAME})")
    parser.add_argument("--port", type=int, default=8000, help="Listening port on 127.0.0.1")
    parser.add_argument("--cuda", choices=("12.9", "13.2"), default="12.9", help="CUDA variant for --download")
    parser.add_argument("--download", action="store_true", help="Download the pinned runtime and default checkpoint if missing")
    args = parser.parse_args()

    if not 1 <= args.port <= 65535:
        parser.error("--port must be between 1 and 65535")
    system = platform.system().lower()
    if system not in {"linux", "windows"} or platform.machine().lower() not in {"x86_64", "amd64"}:
        parser.error("The pinned Albatross release needs an x64 Linux or Windows CUDA host")

    executable = SERVER + (".exe" if system == "windows" else "")
    try:
        bundle = runtime_bundle(args.runtime_dir.expanduser().resolve(), system, executable, args.cuda, args.download)
        model = model_file(args.model.expanduser().resolve(), args.download)
        env = os.environ.copy()
        key = env.pop("ALBATROSS_API_KEY", "")
        if system == "windows":
            env["PATH"] = str(bundle / "lib") + os.pathsep + env.get("PATH", "")
        binary = str(bundle / executable)
        version = subprocess.run([binary, "--version"], cwd=bundle, env=env, check=True, text=True, stdout=subprocess.PIPE)
        if version.stdout.strip() != f"{SERVER} {VERSION}":
            raise ValueError(f"Expected Albatross release {VERSION}, got {version.stdout.strip()!r}")
        command = [binary, "--model-path", str(model), "--vocab-path", str(bundle / VOCAB),
                   "--host", "127.0.0.1", "--port", str(args.port), "--chunk-size", "128", "--chunk-load"]
        if key:
            command += ["--password", key]
        print(f"Starting Albatross on http://127.0.0.1:{args.port}", flush=True)
        return subprocess.run(command, cwd=bundle, env=env).returncode
    except (OSError, ValueError, tarfile.TarError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Albatross launch failed: {error}\n")
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
