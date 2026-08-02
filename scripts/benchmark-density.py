#!/usr/bin/env python3
import argparse
import http.client
import json
import platform
import statistics
import sys
import time
from pathlib import Path
from urllib.parse import urlsplit


DEFAULT_CGROUP_ROOT = Path("/sys/fs/cgroup/ferrobox")


def api(
    endpoint,
    method: str,
    path: str,
    *,
    payload: dict | None = None,
    token: str | None = None,
    expected: tuple[int, ...] = (200,),
) -> dict | None:
    body = None if payload is None else json.dumps(payload, separators=(",", ":"))
    headers = {}
    if body is not None:
        headers["content-type"] = "application/json"
    if token is not None:
        headers["authorization"] = f"Bearer {token}"
    connection = http.client.HTTPConnection(endpoint.hostname, endpoint.port, timeout=30)
    try:
        connection.request(method, path, body=body, headers=headers)
        response = connection.getresponse()
        response_body = response.read()
    finally:
        connection.close()
    if response.status not in expected:
        rendered = response_body.decode(errors="replace")
        raise RuntimeError(
            f"{method} {path}: expected {expected}, received {response.status}: {rendered}"
        )
    if not response_body:
        return None
    return json.loads(response_body)


def meminfo_kib() -> dict[str, int]:
    values = {}
    for line in Path("/proc/meminfo").read_text().splitlines():
        key, raw = line.split(":", 1)
        fields = raw.split()
        if fields:
            values[key] = int(fields[0])
    for required in ("MemTotal", "MemAvailable"):
        if required not in values:
            raise RuntimeError(f"/proc/meminfo omitted {required}")
    return values


def available_samples_kib(count: int = 5) -> list[int]:
    samples = []
    for index in range(count):
        samples.append(meminfo_kib()["MemAvailable"])
        if index + 1 < count:
            time.sleep(0.05)
    return samples


def cgroup_leaves(root: Path) -> list[Path]:
    if not root.exists():
        return []
    return sorted(path for path in root.iterdir() if path.is_dir())


def wait_for_cgroups(root: Path, expected: int, timeout_seconds: float = 10) -> list[Path]:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        leaves = cgroup_leaves(root)
        if len(leaves) == expected:
            return leaves
        time.sleep(0.02)
    observed = [path.name for path in cgroup_leaves(root)]
    raise RuntimeError(
        f"expected {expected} Ferrobox cgroups, observed {len(observed)}: {observed}"
    )


def cgroup_processes(leaves: list[Path]) -> list[int]:
    process_ids = []
    for leaf in leaves:
        process_ids.extend(
            int(value) for value in (leaf / "cgroup.procs").read_text().split()
        )
    return sorted(set(process_ids))


def smaps_rollup_kib(process_id: int) -> dict[str, int]:
    values = {}
    for line in Path(f"/proc/{process_id}/smaps_rollup").read_text().splitlines():
        if ":" not in line:
            continue
        key, raw = line.split(":", 1)
        fields = raw.split()
        if fields and fields[0].isdigit():
            values[key] = int(fields[0])
    for required in ("Rss", "Pss", "Private_Clean", "Private_Dirty"):
        if required not in values:
            raise RuntimeError(f"smaps_rollup for {process_id} omitted {required}")
    values["Uss"] = (
        values["Private_Clean"]
        + values["Private_Dirty"]
        + values.get("Private_Hugetlb", 0)
    )
    return values


def measure_tier(root: Path, sandbox_count: int, baseline_available_kib: int) -> dict:
    leaves = wait_for_cgroups(root, sandbox_count)
    process_ids = cgroup_processes(leaves)
    if not process_ids:
        raise RuntimeError("live Ferrobox cgroups contained no processes")
    process_memory = [smaps_rollup_kib(process_id) for process_id in process_ids]
    cgroup_current_kib = sum(
        int((leaf / "memory.current").read_text().strip()) // 1024 for leaf in leaves
    )
    samples = available_samples_kib()
    available_kib = int(statistics.median(samples))
    host_used_delta_kib = baseline_available_kib - available_kib
    rss_kib = sum(item["Rss"] for item in process_memory)
    pss_kib = sum(item["Pss"] for item in process_memory)
    uss_kib = sum(item["Uss"] for item in process_memory)
    process_names = sorted(
        {Path(f"/proc/{process_id}/comm").read_text().strip() for process_id in process_ids}
    )
    return {
        "sandbox_count": sandbox_count,
        "cgroup_count": len(leaves),
        "firecracker_process_count": len(process_ids),
        "process_names": process_names,
        "host_mem_available_samples_kib": samples,
        "host_mem_available_median_kib": available_kib,
        "host_used_delta_kib": host_used_delta_kib,
        "host_used_delta_per_sandbox_kib": round(
            host_used_delta_kib / sandbox_count, 2
        ),
        "firecracker_rss_kib": rss_kib,
        "firecracker_rss_per_sandbox_kib": round(rss_kib / sandbox_count, 2),
        "firecracker_pss_kib": pss_kib,
        "firecracker_pss_per_sandbox_kib": round(pss_kib / sandbox_count, 2),
        "firecracker_uss_kib": uss_kib,
        "firecracker_uss_per_sandbox_kib": round(uss_kib / sandbox_count, 2),
        "cgroup_memory_current_kib": cgroup_current_kib,
        "cgroup_memory_current_per_sandbox_kib": round(
            cgroup_current_kib / sandbox_count, 2
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", required=True)
    parser.add_argument("--tiers", default="1,5,10,25")
    parser.add_argument("--cgroup-root", type=Path, default=DEFAULT_CGROUP_ROOT)
    parser.add_argument("--minimum-available-mib", type=int, default=2048)
    parser.add_argument("--github-sha", default="")
    arguments = parser.parse_args()

    endpoint = urlsplit(arguments.url)
    if endpoint.scheme != "http" or endpoint.hostname not in {"127.0.0.1", "localhost"}:
        raise ValueError("density benchmark requires a loopback HTTP endpoint")
    tiers = [int(value) for value in arguments.tiers.split(",")]
    if tiers != sorted(set(tiers)) or not tiers or tiers[0] < 1 or tiers[-1] > 32:
        raise ValueError("tiers must be unique ascending integers between 1 and 32")
    if arguments.minimum_available_mib < 512:
        raise ValueError("minimum available memory must be at least 512 MiB")

    initial_leaves = cgroup_leaves(arguments.cgroup_root)
    if initial_leaves:
        raise RuntimeError(
            "density baseline contains Ferrobox cgroups: "
            + ", ".join(path.name for path in initial_leaves)
        )
    baseline_samples = available_samples_kib()
    baseline_available_kib = int(statistics.median(baseline_samples))
    baseline = {
        "cgroup_count": 0,
        "host_mem_total_kib": meminfo_kib()["MemTotal"],
        "host_mem_available_samples_kib": baseline_samples,
        "host_mem_available_median_kib": baseline_available_kib,
    }

    request_body = {
        "template": "python",
        "cpu_count": 1,
        "memory_mb": 512,
        "timeout_seconds": 300,
        "network": {"internet_access": False},
    }
    created = []
    measurements = []
    creation_us = []
    try:
        for tier in tiers:
            available_mib = meminfo_kib()["MemAvailable"] // 1024
            if available_mib < arguments.minimum_available_mib:
                raise RuntimeError(
                    f"host has {available_mib} MiB available before tier {tier}"
                )
            while len(created) < tier:
                started = time.perf_counter_ns()
                handle = api(
                    endpoint,
                    "POST",
                    "/v1/sandboxes",
                    payload=request_body,
                    expected=(201,),
                )
                creation_us.append((time.perf_counter_ns() - started) // 1000)
                created.append(handle)
            wait_for_cgroups(arguments.cgroup_root, tier)
            time.sleep(0.25)
            measurements.append(
                measure_tier(arguments.cgroup_root, tier, baseline_available_kib)
            )
    finally:
        for handle in reversed(created):
            try:
                api(
                    endpoint,
                    "DELETE",
                    f"/v1/sandboxes/{handle['sandbox_id']}",
                    token=handle["token"],
                    expected=(204, 404),
                )
            except Exception as error:
                print(f"density cleanup failed: {error}", file=sys.stderr, flush=True)

    cleanup_leaves = wait_for_cgroups(arguments.cgroup_root, 0)
    cleanup_samples = available_samples_kib()
    print(
        json.dumps(
            {
                "schema_version": 1,
                "github_sha": arguments.github_sha,
                "kernel_release": platform.release(),
                "sandbox_spec": request_body,
                "tiers_requested": tiers,
                "baseline": baseline,
                "tiers": measurements,
                "create_to_ready_us": creation_us,
                "cleanup": {
                    "cgroup_count": len(cleanup_leaves),
                    "host_mem_available_samples_kib": cleanup_samples,
                    "host_mem_available_median_kib": int(
                        statistics.median(cleanup_samples)
                    ),
                },
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
