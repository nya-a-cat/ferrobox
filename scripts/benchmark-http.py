#!/usr/bin/env python3
import argparse
import base64
import concurrent.futures
import http.client
import json
import time
from urllib.parse import quote, urlsplit


def percentile(sorted_samples: list[int], value: int) -> int:
    rank = (len(sorted_samples) * value + 99) // 100
    return sorted_samples[min(rank - 1, len(sorted_samples) - 1)]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", required=True)
    parser.add_argument("--iterations", type=int, default=5)
    parser.add_argument("--concurrency", type=int, default=5)
    arguments = parser.parse_args()
    if (
        arguments.iterations < 1
        or arguments.iterations > 100
        or arguments.concurrency < 1
        or arguments.concurrency > 32
    ):
        raise ValueError("iterations/concurrency are outside supported limits")

    endpoint = urlsplit(arguments.url)
    connection = http.client.HTTPConnection(endpoint.hostname, endpoint.port, timeout=10)
    request_body = json.dumps(
        {
            "template": "python",
            "cpu_count": 1,
            "memory_mb": 512,
            "timeout_seconds": 120,
            "network": {"internet_access": False},
        },
        separators=(",", ":"),
    )

    def create_sandbox(client: http.client.HTTPConnection) -> tuple[dict, int]:
        started = time.perf_counter_ns()
        client.request(
            "POST",
            "/v1/sandboxes",
            body=request_body,
            headers={"content-type": "application/json"},
        )
        response = client.getresponse()
        body = response.read()
        elapsed = (time.perf_counter_ns() - started) // 1000
        if response.status != 201:
            raise RuntimeError(f"create returned {response.status}: {body.decode()}")
        return json.loads(body), elapsed

    def delete_sandbox(client: http.client.HTTPConnection, created: dict) -> None:
        client.request(
            "DELETE",
            f"/v1/sandboxes/{created['sandbox_id']}",
            headers={"authorization": f"Bearer {created['token']}"},
        )
        deleted = client.getresponse()
        deleted_body = deleted.read()
        if deleted.status != 204:
            raise RuntimeError(
                f"delete returned {deleted.status}: {deleted_body.decode()}"
            )

    def create_and_delete(client: http.client.HTTPConnection) -> int:
        created, elapsed = create_sandbox(client)
        delete_sandbox(client, created)
        return elapsed

    samples = [create_and_delete(connection) for _ in range(arguments.iterations)]
    concurrent_started = time.perf_counter_ns()
    with concurrent.futures.ThreadPoolExecutor(
        max_workers=arguments.concurrency
    ) as executor:
        concurrent_samples = list(
            executor.map(
                lambda _: create_and_delete(
                    http.client.HTTPConnection(
                        endpoint.hostname, endpoint.port, timeout=10
                    )
                ),
                range(arguments.concurrency),
            )
        )
    concurrent_wall_us = (time.perf_counter_ns() - concurrent_started) // 1000

    file_sandbox, _ = create_sandbox(connection)
    file_headers = {
        "authorization": f"Bearer {file_sandbox['token']}",
        "content-type": "application/json",
    }
    file_data = b"x" * (1024 * 1024)
    encoded_data = base64.b64encode(file_data).decode()
    write_body = json.dumps(
        {
            "path": "/home/sandbox/ferrobox-api.bin",
            "content_base64": encoded_data,
            "overwrite": True,
        },
        separators=(",", ":"),
    )
    write_samples: list[int] = []
    read_samples: list[int] = []
    read_path = quote("/home/sandbox/ferrobox-api.bin", safe="")
    for _ in range(20):
        started = time.perf_counter_ns()
        connection.request(
            "PUT",
            f"/v1/sandboxes/{file_sandbox['sandbox_id']}/files",
            body=write_body,
            headers=file_headers,
        )
        response = connection.getresponse()
        body = response.read()
        write_samples.append((time.perf_counter_ns() - started) // 1000)
        if response.status != 200 or json.loads(body)["bytes_written"] != len(file_data):
            raise RuntimeError(f"write returned {response.status}: {body.decode()}")

        started = time.perf_counter_ns()
        connection.request(
            "GET",
            f"/v1/sandboxes/{file_sandbox['sandbox_id']}/files?path={read_path}&max_bytes={len(file_data)}",
            headers={"authorization": f"Bearer {file_sandbox['token']}"},
        )
        response = connection.getresponse()
        body = response.read()
        read_samples.append((time.perf_counter_ns() - started) // 1000)
        if response.status != 200:
            raise RuntimeError(f"read returned {response.status}: {body.decode()}")
        result = json.loads(body)
        if (
            result["bytes"] != len(file_data)
            or not result["eof"]
            or base64.b64decode(result["content_base64"]) != file_data
        ):
            raise RuntimeError("read response did not match the uploaded file")
    delete_sandbox(connection, file_sandbox)

    samples.sort()
    concurrent_samples.sort()
    write_samples.sort()
    read_samples.sort()
    print(
        json.dumps(
            {
                "schema_version": 4,
                "http_create_us": samples,
                "http_create_p50_us": percentile(samples, 50),
                "http_create_p95_us": percentile(samples, 95),
                "concurrent_create_us": concurrent_samples,
                "concurrent_create_p50_us": percentile(concurrent_samples, 50),
                "concurrent_create_p95_us": percentile(concurrent_samples, 95),
                "concurrent_create_wall_us": concurrent_wall_us,
                "http_write_1mib_us": write_samples,
                "http_write_1mib_p50_us": percentile(write_samples, 50),
                "http_write_1mib_p95_us": percentile(write_samples, 95),
                "http_read_1mib_us": read_samples,
                "http_read_1mib_p50_us": percentile(read_samples, 50),
                "http_read_1mib_p95_us": percentile(read_samples, 95),
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
