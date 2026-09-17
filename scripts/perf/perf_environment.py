#!/usr/bin/env python3
"""Capture bounded host metadata; never certify a performance profile implicitly."""

import argparse
import datetime
import json
import platform
import re
import subprocess
from pathlib import Path


def command(arguments):
    """Return successful stdout, without retaining potentially sensitive stderr."""
    try:
        result = subprocess.run(arguments, capture_output=True, text=True, timeout=5)
        return result.stdout if result.returncode == 0 else None
    except (OSError, subprocess.TimeoutExpired):
        return None


def power_settings(battery, custom):
    """Extract only the active source and configured low-power mode."""
    source_match = re.search(r"Now drawing from '(AC Power|Battery Power)'", battery or "")
    source = source_match.group(1) if source_match else None
    sections = {}
    current = None
    for line in (custom or "").splitlines():
        if line.strip() in ("AC Power:", "Battery Power:"):
            current = line.strip()[:-1]
        elif current:
            match = re.fullmatch(r"\s*lowpowermode\s+([01])\s*", line)
            if match:
                sections[current] = int(match.group(1))
    return {"source": source, "low_power_mode": sections.get(source)}


def capture(run=command, system=None):
    system = platform.system() if system is None else system
    hardware = {"model": None, "cpu": None, "logical_cpus": None, "memory_bytes": None}
    power = {"source": None, "low_power_mode": None}
    if system == "Darwin":
        for field, key in (("model", "hw.model"), ("cpu", "machdep.cpu.brand_string"),
                           ("logical_cpus", "hw.logicalcpu"), ("memory_bytes", "hw.memsize")):
            value = run(["sysctl", "-n", key])
            if value:
                value = value.strip()
                if field in ("logical_cpus", "memory_bytes"):
                    hardware[field] = int(value) if value.isdecimal() and int(value) > 0 else None
                elif re.fullmatch(r"[A-Za-z0-9 .,()+_-]{1,120}", value):
                    hardware[field] = value
        power = power_settings(run(["pmset", "-g", "batt"]), run(["pmset", "-g", "custom"]))
    return {
        "schema_version": 1,
        "captured_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "os": {"system": system, "release": platform.release(), "architecture": platform.machine()},
        "hardware": hardware,
        "power": power,
        "host_metadata_complete": all(value is not None for value in (*hardware.values(), *power.values())),
        "environment_verified": False,
        "unverified": ["browser and driver versions", "applied CPU/network throttling across frames and workers",
                       "viewport", "server compression", "background workload", "stable power state during samples"],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="New JSON receipt; existing files are never overwritten")
    args = parser.parse_args()
    record = capture()
    with args.output.open("x") as output:
        json.dump(record, output, indent=2)
        output.write("\n")


if __name__ == "__main__":
    main()
