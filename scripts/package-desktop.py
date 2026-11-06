"""Stage two standalone Windows EXEs, or archive other native desktop builds."""
import argparse
import hashlib
import json
import platform
import plistlib
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import zipfile


ROOT = Path(__file__).resolve().parent.parent
HOSTS = {
    ("win32", "amd64"): "windows-x64",
    ("linux", "x86_64"): "linux-x64",
    ("darwin", "arm64"): "macos-arm64",
    ("darwin", "x86_64"): "macos-x64",
}


def command(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def package(args):
    actual = HOSTS.get((sys.platform, platform.machine().lower()))
    if args.platform != actual:
        raise SystemExit(f"Host is {actual or platform.platform()}, cannot package {args.platform} binaries")
    bin_dir = args.bin_dir.resolve()
    out_dir = (args.output_dir or ROOT / "dist" / ("windows-x64-standalone" if sys.platform == "win32" else "")).resolve()
    suffix = ".exe" if sys.platform == "win32" else ""
    names = ["lanmix-sender", "lanmix-receiver"]
    versions = {
        name: tomllib.loads((ROOT / "crates" / name / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
        for name in ("lanmix-sender", "lanmix-receiver")
    }
    workspace_version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    versions = {name: workspace_version if value == {"workspace": True} else value for name, value in versions.items()}
    if len(set(versions.values())) != 1:
        raise SystemExit(f"Desktop versions disagree: {versions}")
    version = versions["lanmix-sender"]
    for name in names:
        binary = bin_dir / (name + suffix)
        if not binary.is_file():
            raise SystemExit(f"Build release binaries first: missing {binary}")
        reported = subprocess.check_output([str(binary), "--version"], text=True).strip().split()[-1]
        if reported != version:
            raise SystemExit(f"Stale {binary.name}: expected {version}, got {reported}")
    commit = command("git", "rev-parse", "HEAD")
    inventory = json.loads((ROOT / "third-party/rust-dependencies.json").read_text(encoding="utf-8"))
    if inventory["cargo_lock_sha256"] != hashlib.sha256((ROOT / "Cargo.lock").read_text(encoding="utf-8").encode("utf-8")).hexdigest():
        raise SystemExit("Dependency license inventory is stale; run scripts/collect-licenses.py")
    if sys.platform == "win32":
        # No duplicate launcher, archive, documentation tree or sidecar DLLs.
        expected = {name + suffix for name in names}
        if out_dir.exists() and any(path.name not in expected or not path.is_file() for path in out_dir.iterdir()):
            raise SystemExit(f"Choose a directory containing only the two desktop EXEs: {out_dir}")
        with tempfile.TemporaryDirectory(prefix="lanmix-notices-") as temporary:
            notice = Path(temporary) / "notices.txt"
            reference = ""
            for name in ("LICENSE", "LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "THIRD_PARTY_NOTICES.md",
                         "vendor/cpal/LICENSE", "vendor/cpal/LANMIX_PATCH.md", "THIRD_PARTY_LICENSES.txt"):
                reference += f"\n================ {name} ================\n\n" + (ROOT / name).read_text(encoding="utf-8") + "\n"
            for name in names:
                subprocess.run([str(bin_dir / (name + suffix)), "--licenses", str(notice)], check=True)
                if notice.read_text(encoding="utf-8") != reference:
                    raise SystemExit(f"Stale embedded redistribution notices in {name}")
        out_dir.mkdir(parents=True, exist_ok=True)
        for name in names:
            artifact = out_dir / (name + suffix)
            shutil.copy2(bin_dir / artifact.name, artifact)
            print(f"Executable: {artifact}\nSHA256: {hashlib.sha256(artifact.read_bytes()).hexdigest()}")
        print(f"Version: {version}\nCommit: {commit}")
        return
    stem = f"LanMix-{version}-{args.platform}-{commit[:8]}"
    out_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="lanmix-package-") as temporary:
        folder = Path(temporary) / stem
        folder.mkdir()
        for name in names:
            shutil.copy2(bin_dir / (name + suffix), folder / (name + suffix))
        shutil.copy2(ROOT / "docs" / "DESKTOP-BUILDS.md", folder / "使用说明.md")
        shutil.copy2(ROOT / "README.md", folder / "README.md")
        shutil.copytree(ROOT / "docs", folder / "docs")
        shutil.copy2(ROOT / "vendor" / "cpal" / "LICENSE", folder / "CPAL-LICENSE")
        for name in ("LICENSE", "LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "THIRD_PARTY_NOTICES.md", "THIRD_PARTY_LICENSES.txt"):
            shutil.copy2(ROOT / name, folder / name)
        shutil.copytree(ROOT / "third-party", folder / "third-party")
        (folder / "BUILD-INFO.json").write_text(json.dumps({
            "version": version, "commit": commit, "platform": args.platform,
            "rustc": command("rustc", "--version"),
            "system_audio_capture": "wasapi-loopback" if sys.platform == "win32" else "explicit-input-only",
        }, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        if sys.platform == "darwin":
            bundle = folder / "LanMix.app"
            contents = bundle / "Contents"
            (contents / "MacOS").mkdir(parents=True)
            shutil.copy2(folder / "lanmix-receiver", contents / "MacOS" / "lanmix-receiver")
            with (contents / "Info.plist").open("wb") as file:
                plistlib.dump({
                    "CFBundleName": "LanMix", "CFBundleDisplayName": "LanMix",
                    "CFBundleIdentifier": "io.lanmix.receiver.desktop",
                    "CFBundleExecutable": "lanmix-receiver", "CFBundlePackageType": "APPL",
                    "CFBundleShortVersionString": version, "CFBundleVersion": version,
                    "LSMinimumSystemVersion": "11.0", "NSHighResolutionCapable": True,
                    "NSLocalNetworkUsageDescription": "发现局域网内的 LanMix 设备并接收、混合它们的音频。",
                }, file)
            # Ad-hoc signing uses no third-party publisher and no developer credentials.
            subprocess.run(["codesign", "--force", "--sign", "-", str(bundle)], check=True)
            subprocess.run(["codesign", "--verify", "--deep", "--strict", str(bundle)], check=True)
            for name in names:
                subprocess.run(["file", str(folder / name)], check=True)
                subprocess.run(["otool", "-L", str(folder / name)], check=True)
        extension = ".tar.gz" if sys.platform == "linux" else ".zip"
        archive = out_dir / (stem + extension)
        if extension == ".tar.gz":
            with tarfile.open(archive, "w:gz") as output:
                output.add(folder, arcname=stem)
        else:
            with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as output:
                for path in sorted(folder.rglob("*")):
                    if path.is_file():
                        output.write(path, path.relative_to(folder.parent))
        checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
        (out_dir / "SHA256SUMS.txt").write_text(f"{checksum}  {archive.name}\n", encoding="utf-8")
        print(f"Package: {archive}\nSHA256: {checksum}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=sorted(HOSTS.values()), required=True)
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target" / "release")
    parser.add_argument("--output-dir", type=Path)
    package(parser.parse_args())


if __name__ == "__main__":
    main()
