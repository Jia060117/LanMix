"""Real cross-process discovery and control-plane/PCM isolation, without opening a UI."""
import argparse
import json
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import uuid


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", type=Path, default=Path("target/release"))
    args = parser.parse_args()
    suffix = ".exe" if __import__("os").name == "nt" else ""
    receiver = args.bin_dir / ("lanmix-receiver" + suffix)
    sender = args.bin_dir / ("lanmix-sender" + suffix)
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as reserve:
        reserve.bind(("127.0.0.1", 0))
        audio_port = reserve.getsockname()[1]
    with tempfile.TemporaryDirectory(prefix="lanmix-discovery-") as temporary:
        report = Path(temporary) / "receiver.json"
        process = subprocess.Popen(
            [str(receiver), "--headless", "--bind", f"127.0.0.1:{audio_port}",
             "--seconds", "9", "--report", str(report)],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        try:
            time.sleep(.5)
            scan = subprocess.run([str(sender), "discover", "--seconds", "3"],
                                  capture_output=True, check=True, timeout=8)
            peers = json.loads(scan.stdout)["peers"]
            peer = next((p for p in peers if p["role"] == "receiver" and p["audio_port"] == audio_port), None)
            assert peer and peer["ready"], ("actual receiver was not discovered", peers)
            # Validate that a live sender can also be discovered by this receiver.
            fake_id = uuid.uuid4().hex
            envelope = {"magic": "LMXD", "protocol": 1, "kind": "probe", "peer": {
                "id": fake_id, "name": "discovery integration", "role": "sender",
                "audio_port": 0, "ready": True, "version": "test"}}
            address = peer["address"].rsplit(":", 1)
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
                probe.bind(("0.0.0.0", 0)); probe.settimeout(2)
                probe.sendto(json.dumps(envelope).encode(), (address[0], int(address[1])))
                reply, _ = probe.recvfrom(2048)
                reply = json.loads(reply)
                assert reply["kind"] == "hello" and reply["peer"]["id"] == peer["id"], reply
            tone = subprocess.run([str(sender), "tone", "--to", f"127.0.0.1:{audio_port}",
                                   "--seconds", "2"], capture_output=True, check=True, timeout=5)
            stdout, stderr = process.communicate(timeout=12)
            assert process.returncode == 0, (stdout, stderr)
            audio = json.loads(report.read_text(encoding="utf-8"))
            assert audio["network"]["invalid_packets"] == 0, audio["network"]
            assert audio["output"]["non_silent_samples"] > 0, audio["output"]
            assert len(audio["tracks"]) == 1, audio["tracks"]
            print(json.dumps({"discovered": peer["name"], "receiver_port": audio_port,
                              "discovery_reply": "hello", "audio_invalid_packets": 0,
                              "audio_sources": 1}, ensure_ascii=False))
            print("PASS: actual processes discover both ways; discovery traffic stays out of PCM")
        finally:
            if process.poll() is None:
                process.terminate(); process.communicate(timeout=5)


if __name__ == "__main__":
    main()
