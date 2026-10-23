"""Measure CPU and peak RSS in fresh Linux processes, with broad regression bounds.

Growth bounds compare the smallest and the largest size of each workload. A
parser that needed twice the memory at every size would pass them, so three
dense workloads also have an absolute bound: one parse of the largest input
must peak below AMPLIFICATION_MAX bytes of RSS per input byte, plus
BASELINE_KIB.

Measured with one parse in a fresh process on Linux x86-64, with the input
string included, the unchanged parser peaks at 66 bytes per input byte for
blank lines (`newlines`), 85 for `=` lines (`error-lines`, an error with its
own message every two bytes), and 110 for `[` lines (`header-errors`, two
errors every two bytes, the densest input found). TESTING.md turns this into
a sizing rule for untrusted input, about 128 bytes per input byte, and
`header-errors` is bounded at that rule. The other two bounds are about one
and a half times what the workload needs. A regression that doubles the
memory of a line takes the three workloads to 130, 169 and 218, at least 30%
above their bounds of 96, 128 and 128, so the check fails.
"""

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

WORKLOADS = (
    "entries",
    "errors",
    "error-lines",
    "header-errors",
    "continuations",
    "long-value",
    "newlines",
)
# Parses per growth sample: enough CPU time to measure at the smallest size.
GROWTH_PARSES = 8
# Bytes of peak RSS per input byte that one parse of a workload's largest
# input may need (module docstring).
AMPLIFICATION_MAX = {"newlines": 96, "error-lines": 128, "header-errors": 128}
# Twice the probe's own peak RSS for a one-unit input, about 2 MiB. The peak
# that wait4 reports for a child also counts the memory of the parent it was
# forked from (Python's, about 14 MiB on CI), but the bounded samples need
# hundreds of MiB, far more than either.
BASELINE_KIB = 4 * 1024


def measure(probe, kind, units, parses):
    # wait4 measures this child, excluding Cargo, other jobs, and time spent
    # waiting for a CPU. A fresh process gives each sample its own peak RSS.
    with tempfile.TemporaryFile(mode="w+") as output:
        child = subprocess.Popen(
            [str(probe), kind, str(units), str(parses)], stdout=output, stderr=subprocess.STDOUT
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
        # The probe echoes its arguments, so a sample cannot describe a
        # different size or parse count than the one requested.
        assert result["units"] == units, result
        assert result["parses"] == parses, result
        result.update(cpu_seconds=usage.ru_utime + usage.ru_stime, peak_rss_kib=usage.ru_maxrss)
        return result


def sample(probe, kind, units, parses, samples):
    # Medians of `samples` fresh processes, which all parse the same input.
    measurements = [measure(probe, kind, units, parses) for _ in range(samples)]
    size_bytes = measurements[0]["bytes"]
    for measurement in measurements:
        assert measurement["bytes"] == size_bytes, measurements
    cpu_seconds = [measurement["cpu_seconds"] for measurement in measurements]
    peak_rss_kib = [measurement["peak_rss_kib"] for measurement in measurements]
    return {
        "workload": kind,
        "units": units,
        "bytes": size_bytes,
        "parses": parses,
        "cpu_seconds": statistics.median(cpu_seconds),
        "peak_rss_kib": statistics.median(peak_rss_kib),
    }


def amplification(probe, kind, units, samples):
    # One parse: parsing again in the same process reuses a fragmented heap,
    # and at some sizes eight parses of `=` lines peak 50% above one.
    row = sample(probe, kind, units, 1, samples)
    row["peak_rss_per_input_byte"] = round(row["peak_rss_kib"] * 1024 / row["bytes"], 1)
    row["limit_kib"] = AMPLIFICATION_MAX[kind] * row["bytes"] // 1024 + BASELINE_KIB
    return row


def check(probe, samples):
    rows = []
    amplifications = []
    failures = []
    for kind in WORKLOADS:
        family = []
        # The unchanged parser has a steep transition between 16K and 64K
        # distinct entries, whose magnitude varies across CI runners. Measure
        # sustained growth above that window instead of gating on the
        # transition. Keep the same growth bound and a fourfold input range.
        sizes = (65_536, 131_072, 262_144) if kind == "entries" else (16_384, 32_768, 65_536)
        for units in sizes:
            row = sample(probe, kind, units, GROWTH_PARSES, samples)
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
        if kind in AMPLIFICATION_MAX:
            row = amplification(probe, kind, sizes[-1], samples)
            amplifications.append(row)
            print(json.dumps(row), flush=True)
            if row["peak_rss_kib"] > row["limit_kib"]:
                failures.append(
                    f"{kind}: one parse peaked at {row['peak_rss_per_input_byte']} bytes of RSS"
                    f" per input byte, above {AMPLIFICATION_MAX[kind]} plus {BASELINE_KIB} KiB"
                )
    return {
        "samples_per_size": samples,
        "measurements": rows,
        "amplification": amplifications,
        "failures": failures,
    }


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
