#!/usr/bin/env python3
import argparse
import http.client
import json
import socket
import time


class UnixConnection(http.client.HTTPConnection):
    def __init__(self, socket_path: str) -> None:
        super().__init__("localhost", timeout=30)
        self.socket_path = socket_path

    def connect(self) -> None:
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(self.timeout)
        self.sock.connect(self.socket_path)


def percentile(sorted_samples: list[int], value: int) -> int:
    rank = (len(sorted_samples) * value + 99) // 100
    return sorted_samples[min(rank - 1, len(sorted_samples) - 1)]


def request(
    connection: UnixConnection,
    method: str,
    path: str,
    body: dict | None = None,
) -> tuple[int, bytes]:
    payload = None if body is None else json.dumps(body, separators=(",", ":"))
    headers = {} if payload is None else {"content-type": "application/json"}
    connection.request(method, path, body=payload, headers=headers)
    response = connection.getresponse()
    return response.status, response.read()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--socket", default="/var/run/docker.sock")
    parser.add_argument("--image", required=True)
    parser.add_argument("--iterations", type=int, default=5)
    parser.add_argument("--exec-iterations", type=int, default=20)
    parser.add_argument("--python-iterations", type=int, default=30)
    parser.add_argument("--file-iterations", type=int, default=20)
    parser.add_argument("--runtime", default="")
    arguments = parser.parse_args()
    if arguments.iterations < 1 or arguments.iterations > 100:
        raise ValueError("iterations must be between 1 and 100")
    if arguments.exec_iterations < 1 or arguments.exec_iterations > 1000:
        raise ValueError("exec iterations must be between 1 and 1000")
    if arguments.python_iterations < 1 or arguments.python_iterations > 1000:
        raise ValueError("python iterations must be between 1 and 1000")
    if arguments.file_iterations < 1 or arguments.file_iterations > 1000:
        raise ValueError("file iterations must be between 1 and 1000")

    connection = UnixConnection(arguments.socket)
    samples: list[int] = []

    def create_container() -> str:
        host_config = {
            "Memory": 512 * 1024 * 1024,
            "NanoCpus": 1_000_000_000,
            "PidsLimit": 512,
            "NetworkMode": "none",
        }
        if arguments.runtime:
            host_config["Runtime"] = arguments.runtime
        status, body = request(
            connection,
            "POST",
            "/v1.44/containers/create",
            {
                "Image": arguments.image,
                "Cmd": ["sleep", "300"],
                "NetworkDisabled": True,
                "HostConfig": host_config,
            },
        )
        if status != 201:
            raise RuntimeError(f"Docker create returned {status}: {body.decode()}")
        return json.loads(body)["Id"]

    def start_container(container_id: str) -> None:
        status, body = request(
            connection,
            "POST",
            f"/v1.44/containers/{container_id}/start",
        )
        if status != 204:
            raise RuntimeError(f"Docker start returned {status}: {body.decode()}")

    def delete_container(container_id: str) -> None:
        status, body = request(
            connection,
            "DELETE",
            f"/v1.44/containers/{container_id}?force=true",
        )
        if status != 204:
            raise RuntimeError(f"Docker delete returned {status}: {body.decode()}")

    for _ in range(arguments.iterations):
        started = time.perf_counter_ns()
        container_id = create_container()
        start_container(container_id)
        samples.append((time.perf_counter_ns() - started) // 1000)
        status, body = request(
            connection,
            "GET",
            f"/v1.44/containers/{container_id}/json",
        )
        if status != 200 or not json.loads(body)["State"]["Running"]:
            raise RuntimeError(f"Docker container did not reach running: {body.decode()}")
        delete_container(container_id)

    samples.sort()
    exec_container_id = create_container()
    start_container(exec_container_id)

    def benchmark_exec(command: list[str], iterations: int) -> list[int]:
        command_samples: list[int] = []
        for _ in range(iterations):
            started = time.perf_counter_ns()
            status, body = request(
                connection,
                "POST",
                f"/v1.44/containers/{exec_container_id}/exec",
                {"Cmd": command, "AttachStdout": False, "AttachStderr": False},
            )
            if status != 201:
                raise RuntimeError(
                    f"Docker exec create returned {status}: {body.decode()}"
                )
            exec_id = json.loads(body)["Id"]
            status, body = request(
                connection,
                "POST",
                f"/v1.44/exec/{exec_id}/start",
                {"Detach": True, "Tty": False},
            )
            if status != 200:
                raise RuntimeError(
                    f"Docker exec start returned {status}: {body.decode()}"
                )
            while True:
                status, body = request(
                    connection,
                    "GET",
                    f"/v1.44/exec/{exec_id}/json",
                )
                if status != 200:
                    raise RuntimeError(
                        f"Docker exec inspect returned {status}: {body.decode()}"
                    )
                state = json.loads(body)
                if not state["Running"]:
                    if state["ExitCode"] != 0:
                        raise RuntimeError(f"Docker exec failed: {state}")
                    break
                time.sleep(0.0005)
            command_samples.append((time.perf_counter_ns() - started) // 1000)
        command_samples.sort()
        return command_samples

    exec_samples = benchmark_exec(["/bin/true"], arguments.exec_iterations)
    benchmark_exec(["python3", "-c", "print(42)"], 1)
    python_samples = benchmark_exec(
        ["python3", "-c", "print(42)"],
        arguments.python_iterations,
    )
    file_command = [
        "python3",
        "-c",
        "from pathlib import Path; p=Path('/tmp/ferrobox-bench.bin'); data=b'x'*1048576; p.write_bytes(data); assert p.read_bytes()==data; p.unlink()",
    ]
    benchmark_exec(file_command, 1)
    file_samples = benchmark_exec(file_command, arguments.file_iterations)
    delete_container(exec_container_id)
    print(
        json.dumps(
            {
                "schema_version": 6,
                "runtime": arguments.runtime or "runc",
                "docker_create_start_us": samples,
                "docker_create_start_p50_us": percentile(samples, 50),
                "docker_create_start_p95_us": percentile(samples, 95),
                "docker_exec_true_us": exec_samples,
                "docker_exec_true_p50_us": percentile(exec_samples, 50),
                "docker_exec_true_p95_us": percentile(exec_samples, 95),
                "docker_exec_true_total_us": sum(exec_samples),
                "docker_exec_true_throughput_milli_ops_per_second": (
                    arguments.exec_iterations * 1_000_000_000 // sum(exec_samples)
                ),
                "docker_exec_python_us": python_samples,
                "docker_exec_python_p50_us": percentile(python_samples, 50),
                "docker_exec_python_p95_us": percentile(python_samples, 95),
                "docker_exec_python_total_us": sum(python_samples),
                "docker_exec_python_throughput_milli_ops_per_second": (
                    arguments.python_iterations
                    * 1_000_000_000
                    // sum(python_samples)
                ),
                "docker_exec_file_roundtrip_us": file_samples,
                "docker_exec_file_roundtrip_p50_us": percentile(file_samples, 50),
                "docker_exec_file_roundtrip_p95_us": percentile(file_samples, 95),
                "docker_exec_file_roundtrip_total_us": sum(file_samples),
                "docker_exec_file_roundtrip_throughput_milli_ops_per_second": (
                    arguments.file_iterations
                    * 1_000_000_000
                    // sum(file_samples)
                ),
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
