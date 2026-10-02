"""Measure CPU and peak RSS in fresh Linux processes, with broad regression bounds."""

import argparse
import json
import os
from pathlib import Path
import signal
import statistics
import subprocess
import sys
import tempfile
import time


def measure(probe, kind, units):
    # wait4 measures this child, excluding Cargo, other jobs, and time spent
    # waiting for a CPU. A fresh process gives each sample its own peak RSS.
    with tempfile.TemporaryFile(mode="w+") as output:
        child = subprocess.Popen(
            [str(probe), kind, str(units)], stdout=output, stderr=subprocess.STDOUT
        )
        deadline = time.monotonic() + 45
        timed_out = False
        while True:
            pid, status, usage = os.wait4(child.pid, os.WNOHANG)
            if pid:
                break
            if time.monotonic() >= deadline:
                # Popen.kill() may poll/reap first, racing our wait4 accounting.
                # An unreaped child keeps its PID reserved until wait4 below.
                os.kill(child.pid, signal.SIGKILL)
                _, status, usage = os.wait4(child.pid, 0)
                timed_out = True
                break
            time.sleep(0.05)
        child.returncode = os.waitstatus_to_exitcode(status)
        output.seek(0)
        text = output.read()
        if timed_out or child.returncode:
            raise RuntimeError(f"{kind}/{units}: timeout={timed_out}, exit={child.returncode}\n{text}")
        result = json.loads(text)
        result.update(cpu_seconds=usage.ru_utime + usage.ru_stime, peak_rss_kib=usage.ru_maxrss)
        return result


def check(probe, samples):
    rows = []
    failures = []
    for kind in ("entries", "errors", "continuations", "long-value"):
        family = []
        # The unchanged parser has a steep transition between 16K and 64K
        # distinct entries, whose magnitude varies across CI runners. Measure
        # sustained growth above that window instead of gating on the
        # transition. Keep the same growth bound and a fourfold input range.
        sizes = (65_536, 131_072, 262_144) if kind == "entries" else (16_384, 32_768, 65_536)
        for units in sizes:
            measurements = [measure(probe, kind, units) for _ in range(samples)]
            row = {
                "workload": kind,
                "units": units,
                "bytes": measurements[0]["bytes"],
                "cpu_seconds": statistics.median(m["cpu_seconds"] for m in measurements),
                "peak_rss_kib": statistics.median(m["peak_rss_kib"] for m in measurements),
            }
            family.append(row)
            rows.append(row)
            print(json.dumps(row), flush=True)
        small, large = family[0], family[-1]
        growth = large["bytes"] / small["bytes"]
        # Generous margins absorb allocator/cache effects and timer precision.
        # They catch gross superlinear regressions, not every possible slowdown.
        if large["cpu_seconds"] > small["cpu_seconds"] * growth * 2.5 + 0.05:
            failures.append(f"{kind}: CPU grew excessively for {growth:.2f}x input")
        if large["peak_rss_kib"] > small["peak_rss_kib"] * growth * 2:
            failures.append(f"{kind}: peak RSS grew excessively for {growth:.2f}x input")
    return {"samples_per_size": samples, "measurements": rows, "failures": failures}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--probe", type=Path, default=Path("target/release/examples/scaling_probe"))
    parser.add_argument("--output", type=Path, default=Path("target/scaling-report.json"))
    parser.add_argument("--samples", type=int, default=3)
    args = parser.parse_args()
    if sys.platform != "linux" or not 1 <= args.samples <= 9:
        parser.error("requires Linux and 1..9 samples")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    try:
        report = check(args.probe.resolve(), args.samples)
    except Exception as error:
        args.output.write_text(json.dumps({"error": str(error)}, indent=2) + "\n")
        raise
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    if report["failures"]:
        raise SystemExit("\n".join(report["failures"]))
