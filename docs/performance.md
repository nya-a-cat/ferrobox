# Performance

Ferrobox performance claims are based on retained GitHub-hosted KVM artifacts.
The benchmark records microseconds as integers and retains all cold-create,
hot-execution, and deletion samples so percentile calculations remain
auditable. Current hosted-KVM runs use five lifecycle samples, one hundred
minimal-command samples, thirty Python samples, and twenty file-workload
samples.

Percentiles use the nearest-rank method. A five-sample P95 is therefore the
maximum sample, which keeps small-sample gates conservative.

## Comparator hierarchy

The primary comparison set is Firecracker, Cloud Hypervisor, Kata Containers,
and E2B:

- Native Firecracker quantifies the VMM and guest boundary under Ferrobox.
- Cloud Hypervisor provides a same-host, independently implemented Rust VMM
  control using the Ferrobox guest protocol.
- Kata Containers measures the complete containerd/CRI, shim, and microVM
  request path and reports its selected VMM separately.
- E2B provides a remote Firecracker product boundary. Network and deployment
  differences prevent a same-host speedup ratio.

Docker/runc and gVisor/runsc remain secondary controls. They show the cost and
security-boundary trade-off against common container execution paths.

## Matched microVM matrix

Run [30432673085](https://github.com/nya-a-cat/ferrobox/actions/runs/30432673085)
is the first same-run matrix containing the full Ferrobox runtime, a
fresh-boot Ferrobox pool, direct Firecracker, Cloud Hypervisor v53.0, and Kata
Containers 3.31.0 with its default QEMU configuration.

| Startup or allocation boundary | P50 | P95 | Samples |
| --- | ---: | ---: | ---: |
| Ferrobox ready-pool HTTP allocation | 1.784 ms | 2.810 ms | 5 |
| Ferrobox snapshot-pool preparation | 961.798 ms | 1,038.732 ms | 5 |
| Ferrobox fresh-pool preparation | 2,926.331 ms | 2,985.318 ms | 5 |
| Direct Firecracker cold launch to guest ready | 1,684.991 ms | 2,501.760 ms | 5 |
| Cloud Hypervisor cold launch to guest ready | 2,518.585 ms | 2,540.687 ms | 5 |
| Kata QEMU complete cold `/bin/true` job | 1,481.160 ms | 1,579.662 ms | 5 |

| Warm runtime | `/bin/true` P50 | `/bin/true` P95 | Python P50 | Python P95 |
| --- | ---: | ---: | ---: | ---: |
| Direct Firecracker guest protocol | 1.908 ms | 5.037 ms | 11.134 ms | 11.676 ms |
| Cloud Hypervisor guest protocol | 1.889 ms | 6.167 ms | 10.604 ms | 11.185 ms |
| Ferrobox snapshot pool, full runtime | 3.119 ms | 17.495 ms | 15.877 ms | 39.557 ms |
| Ferrobox fresh-boot pool, full runtime | 2.861 ms | 5.528 ms | 11.791 ms | 22.026 ms |
| Kata QEMU through containerd and shim-v2 | 31.118 ms | 33.237 ms | 56.028 ms | 61.262 ms |

Every direct-VMM row uses the same kernel, reflink rootfs, static Ferrobox
guest, vsock connector, 100 `/bin/true` samples, and 30 Python samples. Kata
uses the same Python image as the Docker control. Its hot samples are collected
from bounded prewarmed QEMU microVM batches through `ctr`, containerd, shim-v2,
and kata-agent. VM start and cleanup stay outside the hot timers.

The full snapshot-pooled Ferrobox runtime beat Kata QEMU at both percentiles
for both workloads. Its `/bin/true` P50 was 9.98 times lower and Python P50 was
3.53 times lower. The fresh-boot pool improved `/bin/true` P95 from 17.495 ms
to 5.528 ms and Python P50 from 15.877 ms to 11.791 ms. Five fresh guests used
500,240 KiB RSS; five snapshot-restored guests used 338,644 KiB.

The raw VMM data rules out a weak Firecracker execution algorithm as the main
cause. Direct Firecracker and Cloud Hypervisor are within 0.019 ms at
`/bin/true` P50 in this run, and their P95 ordering favors Firecracker.
Ferrobox's remaining gap appears above that boundary: snapshot-backed memory
produces a larger tail, and full runtime bookkeeping adds roughly one
millisecond to the minimal-command median relative to direct Firecracker.
Benchmark schema 10 records the sandbox lookup, state check, and guest-client
clone separately as `guest_lookup_us`. It also splits minimal command execution
into validation, guest lookup, start-RPC, stream, and total durations under
`exec_true_timings`. These diagnostics identify where the remaining median gap
occurs without changing runtime semantics.
MicroVM probe schema 2 adds a second minimal-command series that clones the
tonic client before each request. The persistent-client and cloned-client
series run against the same guest, isolating client reuse from VMM and guest
variation.
The workflow also reruns direct Firecracker inside the same
`cpu.max=100000 100000` host quota used by a one-vCPU Ferrobox sandbox. This
diagnostic separates VMM/runtime implementation cost from host CPU scheduling
policy.

The mandatory microVM leadership step remains red because the snapshot-pooled
full runtime does not yet beat direct Cloud Hypervisor at `/bin/true` P50.
The later HTTP file-API leadership gate also remains red. These gates are kept
as optimization targets.

## Cloud Hypervisor

Run [30427754169](https://github.com/nya-a-cat/ferrobox/actions/runs/30427754169)
is the first retained same-host Cloud Hypervisor comparison:

| Runtime boundary | P50 | P95 | Samples |
| --- | ---: | ---: | ---: |
| Cloud Hypervisor cold launch to guest ready | 2,514.082 ms | 2,530.100 ms | 5 |
| Cloud Hypervisor guest `/bin/true` | 3.111 ms | 7.298 ms | 20 |
| Cloud Hypervisor guest Python 3.11 | 10.617 ms | 11.510 ms | 10 |

The workflow pins Cloud Hypervisor v53.0 and verifies the official static
binary with SHA-256 before execution. It launches one vCPU and 512 MiB of
memory, attaches a reflink clone of the same Python rootfs, boots the same
kernel, and connects through the same hybrid-vsock guest protocol. The first
four launches stop after guest health; the fifth also authenticates and
initializes the guest, executes twenty `/bin/true` requests, performs one
untimed Python warmup, and executes ten timed Python requests.

The Cloud Hypervisor startup measurement includes VMM process launch, complete
Linux boot, vsock connection, and guest health. It is directly comparable to
Ferrobox cold `create_to_ready_us`, which is also about 2.5 seconds on retained
runs. It is deliberately separate from Ferrobox's 348.172 ms snapshot restore
and 4–6 ms ready-pool HTTP allocation boundaries.

The hot-command result demonstrates similar low-millisecond guest control for
both VMMs. The retained Ferrobox Python comparison from another run measured
17.327 ms P50 and 19.596 ms P95; Cloud Hypervisor measured 10.617 ms and
11.510 ms here. Runner generation, sample count, and run timing differ, so this
is an engineering signal rather than a strict VMM ranking. A future matrix run
will interleave both VMMs on one runner for a stronger execution comparison.

The overall workflow conclusion is red because its later HTTP file-API
leadership gate correctly failed. The Cloud Hypervisor install and benchmark
steps completed successfully, and the artifact was uploaded.

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

Run [30423283407](https://github.com/nya-a-cat/ferrobox/actions/runs/30423283407)
is the retained nearest-rank same-host startup and command comparison:

| Metric | P50 | P95 |
| --- | ---: | ---: |
| Ferrobox ready-pool internal allocation | 0.010 ms | 0.016 ms |
| Ferrobox HTTP sandbox create | 1.220 ms | 5.888 ms |
| Ferrobox five-client HTTP burst | 5.133 ms | 7.224 ms |
| Docker Engine container create + start | 76.084 ms | 125.341 ms |
| Ferrobox guest `/bin/true` RPC | 6.500 ms | 22.044 ms |
| Docker Engine `/bin/true` exec-to-exit | 34.115 ms | 35.803 ms |

The five-client Ferrobox burst completed in 25.176 ms. On this runner,
Ferrobox HTTP create was 62.4 times lower at P50 and 21.3 times lower at P95
than Docker Engine create-and-start. Ferrobox command completion was 5.25 times
lower at P50 and 1.62 times lower at P95 than Docker exec-to-exit. The Docker
image was resolved to
`python@sha256:b18992999dbe963a45a8a4da40ac2b1975be1a776d939d098c647482bcad5cba`.
The comparison covers allocation/startup latency for a warm service and cached
template image. It does not equate container and microVM isolation strength.

The same runtime artifact recorded snapshot/Python pool preparation at
768.726 ms P50 and 841.545 ms P95. Five warmed Firecracker processes consumed
319,872 KiB RSS in total, approximately 62.5 MiB per ready microVM. First
Python execution after allocation was 21.694 ms.

[E2B currently advertises 80 ms sandbox startup](https://www.e2b.dev/) on its
product page. Ferrobox's same-host HTTP measurement is below that published
value, while the network and deployment boundaries differ. No cross-cloud
speedup ratio is claimed.

Run [30423815234](https://github.com/nya-a-cat/ferrobox/actions/runs/30423815234)
adds a same-host gVisor `runsc` control:

| Metric | Ferrobox P50 | Ferrobox P95 | gVisor P50 | gVisor P95 |
| --- | ---: | ---: | ---: | ---: |
| HTTP allocation / container create + start | 1.561 ms | 4.597 ms | 129.838 ms | 141.948 ms |
| Warm `/bin/true` exec-to-exit | 11.409 ms | 21.408 ms | 19.690 ms | 25.621 ms |

Ferrobox startup latency was 83.2 times lower at P50 and 30.9 times lower at
P95. Its hot-command latency was 1.73 times lower at P50 and 1.20 times lower
at P95. The control used gVisor `release-20260721.0`, the same cached Python
image and Docker Engine API harness, and the same CPU, memory, PID, and
network-disabled settings. The workflow verified the official release archive
using its published SHA-512 file before installation and retained the exact
version and archive digest.

The run passed every functional and performance step through the gVisor gates.
Its overall conclusion remained red because the later Internet-policy test
again failed DNS resolution with `Temporary failure in name resolution`.

Run [30424454694](https://github.com/nya-a-cat/ferrobox/actions/runs/30424454694)
is the first 100-command matched control after caching guest cgroup
availability:

| Runtime | `/bin/true` P50 | `/bin/true` P95 | Sequential throughput |
| --- | ---: | ---: | ---: |
| Ferrobox | 3.847 ms | 14.002 ms | 164.169 ops/s |
| Docker/runc | 36.392 ms | 40.178 ms | 27.183 ops/s |
| gVisor/runsc | 22.998 ms | 24.884 ms | 43.006 ops/s |

Ferrobox had 9.46 times lower P50 and 2.87 times lower P95 than Docker/runc,
and 5.98 times lower P50 and 1.78 times lower P95 than gVisor/runsc. Its
sequential command throughput was 6.04 times Docker/runc and 3.82 times
gVisor/runsc. The complete 100-sample arrays are retained in each JSON
artifact.

The same run measured Ferrobox HTTP allocation at 1.272 ms P50 and 4.212 ms
P95, Docker/runc create-and-start at 84.398 ms P50 and 133.117 ms P95, and
gVisor/runsc create-and-start at 134.211 ms P50 and 149.404 ms P95. The
five-client Ferrobox allocation burst had a 6.234 ms P95 and completed in
23.857 ms.

Run [30424993453](https://github.com/nya-a-cat/ferrobox/actions/runs/30424993453)
adds a 30-sample Python 3.11 short-script comparison after one untimed warmup:

| Runtime | Python P50 | Python P95 | Sequential throughput |
| --- | ---: | ---: | ---: |
| Ferrobox | 17.327 ms | 19.596 ms | 60.742 ops/s |
| Docker/runc | 46.769 ms | 48.847 ms | 21.349 ops/s |
| gVisor/runsc | 40.007 ms | 41.851 ms | 24.952 ops/s |

Ferrobox had 2.70 times lower Python P50 and 2.49 times lower Python P95 than
Docker/runc, and 2.31 times lower P50 and 2.14 times lower P95 than
gVisor/runsc. Its Python throughput was 2.85 times Docker/runc and 2.43 times
gVisor/runsc. The workflow enforces both P95 comparisons as mandatory gates.
All three controls execute `python3 -c "print(42)"` with Python 3.11 after one
untimed interpreter warmup.

The same artifact retained one hundred `/bin/true` samples. Ferrobox measured
3.149 ms P50, 20.238 ms P95, and 150.955 ops/s. Docker/runc measured
35.478 ms, 37.945 ms, and 28.043 ops/s; gVisor/runsc measured 21.139 ms,
22.975 ms, and 46.820 ops/s. This repeat confirms leadership while showing
hosted-runner variation in the Ferrobox upper tail.

Runs [30425633448](https://github.com/nya-a-cat/ferrobox/actions/runs/30425633448)
and [30425743192](https://github.com/nya-a-cat/ferrobox/actions/runs/30425743192)
independently passed a 1 MiB file-roundtrip leadership gate. The latest run
recorded:

| Runtime | File P50 | File P95 | Sequential throughput |
| --- | ---: | ---: | ---: |
| Ferrobox | 20.049 ms | 30.875 ms | 44.558 ops/s |
| Docker/runc | 49.542 ms | 51.994 ms | 20.051 ops/s |
| gVisor/runsc | 45.934 ms | 48.097 ms | 21.724 ops/s |

Ferrobox had 2.47 times lower P50 and 1.68 times lower P95 than Docker/runc,
and 2.29 times lower P50 and 1.56 times lower P95 than gVisor/runsc. Its
sequential throughput was 2.22 times Docker/runc and 2.05 times gVisor/runsc.

The preceding independent run measured Ferrobox at 25.485 ms P50 and
26.565 ms P95, Docker/runc at 61.809 ms and 64.381 ms, and gVisor/runsc at
57.950 ms and 59.198 ms. Both runs passed the mandatory final P95 gate, which
provides initial cross-run evidence while preserving the raw arrays for
variance analysis.

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

The Docker command control runs one hundred `/bin/true` requests in one warm
container. Each timer covers exec creation, detached start, and Engine API
inspection until exit. The workflow compares its P95 with Ferrobox's one hundred
hot `/bin/true` guest RPC samples.

Sequential throughput divides the command count by the complete timed loop.
It includes the same request-to-exit work represented by the individual
samples and does not claim parallel command capacity.

The gVisor control reuses that Docker Engine harness with `HostConfig.Runtime`
set to `runsc`. This isolates the runtime change while preserving the image,
request path, resource limits, sample counts, and timing boundaries. Ferrobox
uses a prewarmed microVM pool, while each gVisor startup sample creates and
starts a container. The table therefore supports a warm-service allocation
claim and does not represent cold host initialization.

The Python comparison executes one untimed warmup followed by thirty timed
requests in each already-running sandbox or container. Every sample includes
runtime RPC/API handling, Python process creation, interpreter startup, script
execution, output handling, and exit observation. It measures repeated short
Python jobs inside warm environments.

The file workload also starts with one untimed warmup. Each of twenty timed
requests launches Python 3.11, allocates a 1 MiB byte string, writes it to the
runtime's `/tmp` filesystem, reads and verifies the complete contents, and
deletes the file. The timer includes runtime RPC/API handling, process and
interpreter startup, filesystem work, and exit observation. Ferrobox uses its
ext4 rootfs over Firecracker virtio-blk, Docker/runc uses the runner's Docker
storage path, and gVisor/runsc uses its configured filesystem path.

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
