"""Real sender -> UDP loss proxy -> headless receiver; no sound device or UI."""
import argparse
import json
from pathlib import Path
import socket
import statistics
import struct
import subprocess
import sys
import tempfile
import time


def run_case(bin_dir, repair_ms):
    suffix = ".exe" if sys.platform == "win32" else ""
    flags = subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0
    children = []
    dropped = {100, 101, 102, 300, 301, 302}
    with tempfile.TemporaryDirectory(prefix="lanmix-repair-") as temp, socket.socket(
        socket.AF_INET, socket.SOCK_DGRAM
    ) as proxy, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as reservation:
        proxy.bind(("127.0.0.1", 0))
        proxy.settimeout(0.05)
        reservation.bind(("127.0.0.1", 0))
        receiver_address = reservation.getsockname()
        reservation.close()
        report_path = Path(temp) / "report.json"
        try:
            receiver = subprocess.Popen(
                [str(bin_dir / ("lanmix-receiver" + suffix)), "--headless", "--bind",
                 f"127.0.0.1:{receiver_address[1]}", "--seconds", "4", "--report", str(report_path)],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8",
                creationflags=flags,
            )
            children.append(receiver)
            assert "Headless mixer listening" in receiver.stdout.readline(), "receiver startup"
            sender = subprocess.Popen(
                [str(bin_dir / ("lanmix-sender" + suffix)), "tone", "--to",
                 f"127.0.0.1:{proxy.getsockname()[1]}", "--repair-ms", str(repair_ms),
                 "--seconds", "3"], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                text=True, encoding="utf-8", creationflags=flags,
            )
            children.append(sender)
            first = {}
            duplicates = 0
            delays = []
            dropped_count = 0
            last_packet = time.perf_counter()
            deadline = last_packet + 8
            while time.perf_counter() < deadline:
                try:
                    packet, _ = proxy.recvfrom(1201)
                except socket.timeout:
                    if sender.poll() is not None and time.perf_counter() - last_packet > 0.15:
                        break
                    continue
                now = time.perf_counter()
                last_packet = now
                seq = struct.unpack_from("<I", packet, 16)[0]
                if seq in first:
                    original, at = first[seq]
                    assert packet == original, "backup changed the header or PCM"
                    duplicates += 1
                    delays.append(now - at)
                else:
                    first[seq] = (packet, now)
                    if seq in dropped:
                        dropped_count += 1
                        continue
                proxy.sendto(packet, receiver_address)
            else:
                raise AssertionError("sender/proxy timed out")
            sender_out, sender_err = sender.communicate(timeout=5)
            assert sender.returncode == 0, (sender_out, sender_err)
            receiver_out, receiver_err = receiver.communicate(timeout=6)
            assert receiver.returncode == 0, (receiver_out, receiver_err)
            report = json.loads(report_path.read_text(encoding="utf-8"))
            track = report["tracks"][0]
            assert dropped_count == 6, dropped_count
            assert report["network"]["invalid_packets"] == 0, report
            assert report["output"]["non_silent_samples"] > 100000, report
            assert report["output"]["clipped_frames"] == 0, report
            if repair_ms:
                assert duplicates > 1000, duplicates
                assert statistics.median(delays) >= repair_ms / 1000 - 0.003, delays[:20]
                assert track["concealed_frames"] == track["missing_datagrams"] == 0, track
                assert track["gap_filled_packets"] >= 6, track
                assert track["duplicate_packets"] > 1000, track
            else:
                assert duplicates == 0, duplicates
                assert track["concealed_frames"] == 6 * 96, track
                assert track["missing_datagrams"] == 6, track
            return {"repair_ms": repair_ms, "injected_lost_originals": dropped_count,
                    "byte_identical_copies": duplicates,
                    "median_copy_delay_ms": round(statistics.median(delays) * 1000, 2) if delays else None,
                    "concealed_frames": track["concealed_frames"],
                    "gap_filled_packets": track["gap_filled_packets"],
                    "missing_datagrams": track["missing_datagrams"]}
        finally:
            for child in children:
                if child.poll() is None:
                    child.kill()
                    child.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=Path("target/release"))
    args = parser.parse_args()
    for repair_ms in (0, 20):
        print(json.dumps(run_case(args.bin_dir.resolve(), repair_ms)))
    print("PASS: actual sender backups repair two 6 ms UDP loss bursts before rendering")


if __name__ == "__main__":
    main()
