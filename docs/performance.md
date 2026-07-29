# Performance

Ferrobox performance claims are based on retained GitHub-hosted KVM artifacts.
The benchmark records microseconds as integers and retains all cold-create,
hot-execution, and deletion samples so percentile calculations remain
auditable. Current hosted-KVM runs use five lifecycle samples and twenty hot
execution samples.

Percentiles use the nearest-rank method. A five-sample P95 is therefore the
maximum sample, which keeps small-sample gates conservative.

## Baseline

Run [30417577685](https://github.com/nya-a-cat/ferrobox/actions/runs/30417577685)
on `ubuntu-24.04` established the first baseline:

| Metric | Result |
| --- | ---: |
| create to guest ready | 2,384.304 ms |
| `/bin/true` P50 | 2.237 ms |
| `/bin/true` P95 | 7.544 ms |
| Python `print(42)` | 16.247 ms |
| delete | 3.228 ms |
| full measured lifecycle | 2,471.580 ms |

The run failed its initial two-second create ceiling and successfully retained
the JSON artifact. The failure is evidence that the gate is active.

Run [30417866400](https://github.com/nya-a-cat/ferrobox/actions/runs/30417866400)
measured the first channel-reuse revision:

| Metric | Baseline | Channel reuse |
| --- | ---: | ---: |
| create to guest ready | 2,384.304 ms | 2,540.730 ms |
| `/bin/true` P50 | 2.237 ms | 3.208 ms |
| `/bin/true` P95 | 7.544 ms | 3.868 ms |
| Python `print(42)` | 16.247 ms | 35.464 ms |
| delete | 3.228 ms | 1.582 ms |

The hot-command P95 improved by 48.7%. Single-sample create and Python results
show runner noise, so future cold-start evidence uses repeated samples before
claiming a latency improvement.

Run [30419522732](https://github.com/nya-a-cat/ferrobox/actions/runs/30419522732)
is the first schema-v2 repeated lifecycle sample:

| Metric | P50 | P95 |
| --- | ---: | ---: |
| create to guest ready | 2,532.207 ms | 2,535.104 ms |
| `/bin/true` | 3.270 ms | 7.258 ms |
| delete | 1.410 ms | 1.498 ms |

The artifact retains all five create/delete samples and all twenty command
samples. The performance step passed its regression ceilings. The workflow
conclusion is red because the later, separately measured Internet DNS test
failed; this does not change the retained benchmark result.

Run [30420163545](https://github.com/nya-a-cat/ferrobox/actions/runs/30420163545)
is the first ready-state snapshot restore result:

| Metric | Cold P95 | Snapshot P50 | Snapshot P95 |
| --- | ---: | ---: | ---: |
| create to guest ready | 2,535.104 ms | 347.688 ms | 348.172 ms |

Snapshot restore reduced create-to-ready P95 by 86.3%. This remains above the
first competitive target, so the project does not claim a startup-latency lead
from this result.

Run [30422952630](https://github.com/nya-a-cat/ferrobox/actions/runs/30422952630)
is the retained nearest-rank same-host control-plane comparison:

| Metric | P50 | P95 |
| --- | ---: | ---: |
| Ferrobox ready-pool internal allocation | 0.012 ms | 0.017 ms |
| Ferrobox HTTP sandbox create | 1.887 ms | 2.742 ms |
| Ferrobox five-client HTTP burst | 4.871 ms | 6.534 ms |
| Docker Engine container create + start | 159.892 ms | 672.967 ms |

The five-client Ferrobox burst completed in 15.627 ms. On this runner,
Ferrobox HTTP create was 84.7 times lower at P50 and 245.4 times lower at P95
than Docker Engine create-and-start. The raw samples are retained because the
Docker control had substantial tail variance. The Docker image was resolved to
`python@sha256:b18992999dbe963a45a8a4da40ac2b1975be1a776d939d098c647482bcad5cba`.
The comparison covers allocation/startup latency for a warm service and cached
template image. It does not equate container and microVM isolation strength.

The same runtime artifact recorded snapshot/Python pool preparation at
572.306 ms P50 and 640.017 ms P95. Five warmed Firecracker processes consumed
326,852 KiB RSS in total, approximately 63.8 MiB per ready microVM. First
Python execution after allocation was 69.791 ms.

[E2B currently advertises 80 ms sandbox startup](https://www.e2b.dev/) on its
product page. Ferrobox's same-host HTTP measurement is below that published
value, while the network and deployment boundaries differ. No cross-cloud
speedup ratio is claimed.

## Measurement boundary

Each `create_to_ready_us` sample starts before `FirecrackerRuntime::create` and
stops only after the guest health check and authenticated initialization
complete. It includes jail preparation, kernel/rootfs cloning, Jailer startup,
Firecracker configuration, Linux boot, vsock connection, guest health, and
initialization. The regression gate uses the retained create P95.

Hot execution includes host-side lookup, gRPC over virtio-vsock, guest process
spawn, process cgroup assignment, exit collection, and response delivery.

The benchmark excludes workflow checkout, compilation, rootfs construction,
and artifact upload.

The KVM benchmark places template images, snapshot assets, and jail roots on
one temporary Btrfs volume. The workflow verifies `cp --reflink=always` before
measurement. This matches the runtime's COW storage requirement and prevents a
full sparse-image copy from being counted as sandbox allocation.

`pool_prepare_us` measures a complete snapshot restore, guest health check, and
unique guest initialization for each prepared microVM. It also warms the Python
interpreter so its main lazy snapshot page faults stay in the background
boundary. `create_to_ready_us` measures a compatible API allocation from that
ready pool. Both sample sets are retained; pool construction is never hidden
inside workflow setup time. Once a snapshot exists, missing pool entries
restore concurrently.

The runtime artifact also records the ready-pool size and summed `VmRSS` from
the corresponding Firecracker processes after native/Python page warmup. This
keeps latency gains paired with an auditable resident-memory cost.

`http_create_us` is the user-facing same-host boundary. It starts immediately
before an HTTP `POST /v1/sandboxes` and ends after the complete 201 response is
read. It includes request parsing, pool allocation, state registration, token
issuance, audit persistence, JSON serialization, and loopback transport.
`concurrent_create_us` applies the same boundary to five simultaneous clients;
`concurrent_create_wall_us` records the complete burst.

The Docker control uses the same host and direct HTTP rather than CLI process
startup. Its timer covers Docker Engine container create and start responses,
with the Python image pulled beforehand. CPU, memory, PID, and network-disabled
limits match the Ferrobox benchmark shape where the Docker API supports them.
Container inspection and deletion occur outside the timed interval.

The Docker command control runs twenty `/bin/true` requests in one warm
container. Each timer covers exec creation, detached start, and Engine API
inspection until exit. The workflow compares its P95 with Ferrobox's twenty
hot `/bin/true` guest RPC samples.

## Targets

The optimization program tracks two different thresholds:

1. Regression ceilings derived from Ferrobox hosted-KVM history.
2. Competitive targets derived from a reproducible same-host harness.

The first competitive restore-to-ready target is below 200 ms. Ready-pool HTTP
allocation targets P95 below 80 ms, matching the current E2B product-page
startup value. Hot `/bin/true` still targets P50 below 1 ms and P95 below 2 ms.
Each target becomes a Ferrobox claim only after a retained artifact passes.

## Optimization order

1. Avoid immutable kernel copies and reuse established vsock/HTTP2 channels.
2. Capture a ready Firecracker memory/device snapshot.
3. Restore from the snapshot with a per-sandbox writable COW rootfs.
4. Add a bounded ready pool for allocation without restore latency.
5. Add concurrent-create throughput and host RSS measurements.

The hosted-KVM workflow prepares the ready-state snapshot during its functional
sandbox, restores and initializes five ready-pool entries, and then measures
five allocations. A competitive claim is added only after both sample sets
pass and the retained artifact shows the result.
