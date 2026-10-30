"""Exercise two actual CLI senders and the actual UDP/DSP receiver, silently."""
import argparse
import json
from pathlib import Path
import socket
import subprocess
import sys
import tempfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", type=Path, default=Path("target/release"))
    args = parser.parse_args()
    suffix = ".exe" if sys.platform == "win32" else ""
    receiver = (args.bin_dir / ("lanmix-receiver" + suffix)).resolve()
    sender = (args.bin_dir / ("lanmix-sender" + suffix)).resolve()
    if not receiver.is_file() or not sender.is_file():
        parser.error("build both release binaries first")
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    flags = subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0
    children = []
    with tempfile.TemporaryDirectory(prefix="lanmix-smoke-") as temp:
        report_path = Path(temp) / "report.json"
        try:
            process = subprocess.Popen(
                [str(receiver), "--headless", "--bind", f"127.0.0.1:{port}", "--seconds", "7", "--report", str(report_path)],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8", creationflags=flags,
            )
            children.append(process)
            # Readiness is printed after socket binding and mixer construction.
            ready = process.stdout.readline()
            if "Headless mixer listening" not in ready:
                raise AssertionError(f"receiver did not start: {ready} {process.stderr.read()}")
            for stream_id, frequency, rate in [(1, 440, 44100), (2, 660, 96000)]:
                child = subprocess.Popen([
                    str(sender), "tone", "--to", f"127.0.0.1:{port}",
                    "--frequency", str(frequency), "--sample-rate", str(rate),
                    "--stream-id", str(stream_id), "--seconds", "5",
                ], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8", creationflags=flags)
                children.append(child)
            for child in children[1:]:
                out, err = child.communicate(timeout=20)
                assert child.returncode == 0, (out, err)
            out, err = process.communicate(timeout=20)
            assert process.returncode == 0, (out, err)
            report = json.loads(report_path.read_text(encoding="utf-8"))
            assert len(report["tracks"]) == 2, report
            assert {t["sample_rate"] for t in report["tracks"]} == {44100, 96000}, report
            assert all(t["packets"] > 500 for t in report["tracks"]), report
            assert report["network"]["invalid_packets"] == 0, report
            assert report["output"]["non_silent_samples"] > 100000, report
            assert 0.01 < report["output"]["peak"] < 0.2, report
            assert 0.005 < report["output"]["rms"] < 0.1, report
            assert report["output"]["clipped_frames"] == 0, report
            print(json.dumps(report, indent=2))
            print("PASS: two source rates -> real UDP -> independent resampling -> finite, non-silent mix")
        finally:
            for child in children:
                if child.poll() is None:
                    child.kill()
                    child.wait(timeout=5)


if __name__ == "__main__":
    main()
