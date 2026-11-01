"""Collect pinned Windows/Android Rust dependency notices from the Cargo sources."""
import concurrent.futures
import hashlib
import json
from pathlib import Path
import re
import subprocess
import struct
import urllib.request
import urllib.error

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "third-party"


def font_notices(path):
    """Read copyright/license records from the shipped sfnt font itself."""
    data = path.read_bytes()
    tables = struct.unpack_from(">H", data, 4)[0]
    for index in range(tables):
        tag, _, start, _ = struct.unpack_from(">4sIII", data, 12 + index * 16)
        if tag != b"name":
            continue
        _, count, strings = struct.unpack_from(">HHH", data, start)
        records = set()
        for index in range(count):
            platform, _, _, name, length, offset = struct.unpack_from(">6H", data, start + 6 + index * 12)
            if name not in (0, 13, 14):
                continue
            raw = data[start + strings + offset:start + strings + offset + length]
            records.add(raw.decode("utf-16-be" if platform in (0, 3) else "mac_roman"))
        return "\n\n".join(sorted(records))
    raise RuntimeError(f"No font name table: {path.name}")


def metadata(target):
    return json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--offline", "--format-version", "1",
        "--filter-platform", target,
    ], cwd=ROOT, encoding="utf-8"))


def collect():
    packages = {}
    for target, names in (
        ("x86_64-pc-windows-msvc", ("lanmix-sender", "lanmix-receiver")),
        ("aarch64-linux-android", ("lanmix-android",)),
        ("armv7-linux-androideabi", ("lanmix-android",)),
    ):
        data = metadata(target)
        nodes = {n["id"]: n for n in data["resolve"]["nodes"]}
        by_id = {p["id"]: p for p in data["packages"]}
        seen = set()

        def walk(identity):
            if identity in seen:
                return
            seen.add(identity)
            package = by_id[identity]
            if not package["name"].startswith("lanmix-"):
                item = packages.setdefault(identity, {"package": package, "targets": []})
                item["targets"].append(target)
            for dependency in nodes[identity]["deps"]:
                if any(kind["kind"] != "dev" for kind in dependency["dep_kinds"]):
                    walk(dependency["pkg"])

        for package in data["packages"]:
            if package["name"] in names:
                walk(package["id"])

    sources = {}

    def fetch(url):
        # Cache upstream texts in the repository for repeatable offline rebuilds.
        name = hashlib.sha256(url.encode()).hexdigest()[:16] + ".txt"
        file = OUTPUT / "upstream" / name
        if not file.exists():
            request = urllib.request.Request(url, headers={"User-Agent": "LanMix-license-audit"})
            with urllib.request.urlopen(request, timeout=30) as response:
                content = response.read()
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_bytes(content)
        sources[name] = url
        return file.read_text(encoding="utf-8"), url

    def notices(item):
        package = item["package"]
        root = Path(package["manifest_path"]).parent
        found = []
        for path in sorted(root.rglob("*")):
            if path.is_file() and re.search(r"license|notice|copying|copyright", path.name, re.I):
                found.append((path.read_text(encoding="utf-8", errors="replace"), str(path.relative_to(root)).replace("\\", "/")))
        vcs_file = root / ".cargo_vcs_info.json"
        vcs = json.loads(vcs_file.read_text()) if vcs_file.exists() else {}
        sha = vcs.get("git", {}).get("sha1")
        repository = re.match(r"https?://github.com/([^/]+/[^/]+)", package.get("repository") or "")
        if not found:
            if not repository or not sha:
                raise RuntimeError(f"Missing upstream license: {package['name']}")
            repo = repository[1].removesuffix(".git")
            if package["name"] == "clipboard-win":
                candidates = ["LICENSE_1_0.txt", "LICENSE", "LICENSE.txt"]
            elif package["name"] == "hexf-parse":
                candidates = ["README.md"]
            else:
                candidates = ["LICENSE-MIT", "LICENSE-APACHE", "LICENSE", "LICENSE.md", "LICENSE.txt"]
            for candidate in candidates:
                try:
                    found.append(fetch(f"https://raw.githubusercontent.com/{repo}/{sha}/{candidate}"))
                except urllib.error.HTTPError as error:
                    if error.code != 404:
                        raise
            if not found:
                raise RuntimeError(f"No pinned license text found: {package['name']}")
        if package["name"] == "epaint_default_fonts":
            for path in sorted((root / "fonts").glob("*.txt")):
                if not re.search(r"license|notice|copying|copyright", path.name, re.I):
                    found.append((path.read_text(encoding="utf-8"), "fonts/" + path.name))
            for path in sorted((root / "fonts").glob("*.ttf")):
                found.append((font_notices(path), "fonts/" + path.name + " (embedded copyright/license records)"))
            for font in ("OFL.txt", "UFL.txt"):
                found.append(fetch(f"https://raw.githubusercontent.com/emilk/egui/{sha}/crates/epaint_default_fonts/fonts/{font}"))
            for license in ("LICENSE-MIT", "LICENSE-APACHE"):
                found.append(fetch(f"https://raw.githubusercontent.com/emilk/egui/{sha}/{license}"))
        return found

    inventory = []
    text = ["LanMix third-party license texts\n\nCopyright and license notices belong to their respective authors.\n"]
    ordered = sorted(packages.values(), key=lambda item: (item["package"]["name"], item["package"]["version"]))
    with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
        results = list(pool.map(notices, ordered))
    for item, found in zip(ordered, results):
        p = item["package"]
        if p["license"] is None:
            raise RuntimeError(f"Unclassified license: {p['name']}")
        record = {key: p[key] for key in ("name", "version", "license", "repository", "authors")}
        record["targets"] = sorted(item["targets"])
        record["notice_sources"] = [source for _, source in found]
        inventory.append(record)
        text.append(f"\n{'=' * 72}\n{p['name']} {p['version']}\nLicense: {p['license']}\nRepository: {p['repository']}\n")
        for content, source in found:
            text.append(f"\n--- {source} ---\n{content.rstrip()}\n")
    # These runtime dependencies are resolved by Gradle rather than Cargo.
    android_sources = json.loads((OUTPUT / "android-sources.json").read_text(encoding="utf-8"))
    text.append("\n" + "=" * 72 + "\nAndroid runtime: Kotlin stdlib 2.2.10 and JetBrains annotations 13.0\nLicense: Apache-2.0\n")
    for filename, url in android_sources.items():
        if filename == "CC0-1.0.txt":
            continue
        text.append(f"\n--- {url} ---\n{(OUTPUT / 'upstream' / filename).read_text(encoding='utf-8').rstrip()}\n")
    text.append("\n" + "=" * 72 + "\nCC0-1.0 full text (hexf-parse)\n" +
                (OUTPUT / "upstream/CC0-1.0.txt").read_text(encoding="utf-8"))
    OUTPUT.mkdir(exist_ok=True)
    snapshot = {
        "cargo_lock_sha256": hashlib.sha256((ROOT / "Cargo.lock").read_text(encoding="utf-8").encode("utf-8")).hexdigest(),
        "cargo_lock_hash_format": "UTF-8 with LF line endings",
        "scope": "Conservative target-filtered dependency closure, including build dependencies; not a binary symbol inventory.",
        "packages": inventory,
    }
    (OUTPUT / "rust-dependencies.json").write_text(json.dumps(snapshot, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    (OUTPUT / "upstream-sources.json").write_text(json.dumps(dict(sorted(sources.items())), indent=2) + "\n", encoding="utf-8")
    (ROOT / "THIRD_PARTY_LICENSES.txt").write_text("".join(text), encoding="utf-8")
    print(f"Collected {len(inventory)} Rust packages and {sum(len(r) for r in results)} license/notice texts")


if __name__ == "__main__":
    collect()
