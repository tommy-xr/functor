---
name: oculus-profiling
description: Profile Functor on an adb-attached Meta Quest with ovrgpuprofiler to find the actual GPU bottleneck — vertex vs fragment split, texture-fetch stalls, cache misses, overdraw — and publish the results as a web-viewable report. Use when a Quest workload misses its frame budget, when deciding between optimizations, or when a performance claim needs evidence rather than inference.
---

# Oculus GPU profiling

`npm run bench:quest` answers *how fast*; this skill answers *why*. VrApi
telemetry gives one aggregate `App` GPU time, which is enough to know you missed
the budget and useless for deciding what to fix. `ovrgpuprofiler` reads Adreno's
hardware counters and splits that number apart.

Reach for this **before** building an optimization, not after. The cost of a
profiling session is one adb command; the cost of optimizing the wrong stage is
a rebuild cycle plus a wrong conclusion in the PR history.

## Profile on desktop *and* device — they measure different things

Desktop benchmarks cannot see GPU cost. `frame_bench` is headless: it measures
CPU allocations and per-frame work, and is blind to shading, texture sampling,
filtering and fill rate. A rendering change can show **exact** `frame_bench`
parity and still be a large GPU regression.

The worked example this skill came from: #477 requested 8× anisotropy for every
mipmapped texture. `allocs/frame` and `bytes/frame` were byte-identical to main
— a clean desktop result — and on a Quest 3 it cost **8.9 ms of a 13.8 ms
frame**, dropping the terrain sample to 44 fps. No desktop tool available to us
would have caught it.

So for anything touching rendering internals, shaders, materials, or sampler
state, run both:

| Where | Tool | Answers |
| --- | --- | --- |
| Desktop | `frame_bench` | CPU allocations, per-frame work |
| Desktop | `npm run test:golden` | did the image change |
| Device | `npm run bench:quest` | frame time, stale frames, budget |
| Device | `ovrgpuprofiler` | *why* — which stage, which stall |

Whenever a Quest is attached, the device numbers are part of done. When one
isn't, say so explicitly in the PR — an unmeasured GPU cost should be visible as
a gap, not mistaken for an absent one.

## Prerequisites

Get a real workload running first — see the **`vr-device-loop` skill** for build,
install, XR-session recovery, and pushing a project. Profiling a dozing session
or a fallback asset measures nothing.

Two traps that silently invalidate a run, both learned the hard way:

- **Assets stream asynchronously.** `functor run vr` pushes source, then streams
  assets. Benchmark or profile too early and you measure the *fallback* — a flat
  2×2 heightmap, a checkerboard texture. Wait **≥18 s** after the push and verify
  before trusting a number. A baseline that looks impossibly good usually is.
- **Use a release APK** and confirm the package is not `DEBUGGABLE`
  (`adb shell dumpsys package dev.functor.runner | grep flags=`).

## Capture

The binary ships on the device at `/system_ext/bin/ovrgpuprofiler`; no
instrumentation in our code, no rebuild.

```sh
D=<serial>
adb -s $D shell ovrgpuprofiler -m                      # list all 81 counters
adb -s $D shell 'ovrgpuprofiler --realtime="24,31"'    # stream, one sample/sec
```

`--realtime` takes a comma-separated list of counter IDs and prints a block per
second until interrupted; wrap it in `timeout` so it can't wedge. Counters are
stable across samples on a steady workload — if they aren't, the scene isn't
settled.

### The counters that answer real questions

**Which stage owns the frame** — the first thing to measure, because it decides
whether fill-rate work (foveation, fewer fetches per fragment) or geometry work
(fewer vertices, fewer fetches per vertex) is worth anything:

| ID | Counter |
| --- | --- |
| 24 | % Time Shading Vertices |
| 31 | % Time Shading Fragments |

**Whether the GPU is computing or waiting.** `% Shaders Busy` high with
`% Shader ALU Capacity Utilized` low means the cores are stalled, not working —
and then ALU optimizations are pointless:

| ID | Counter |
| --- | --- |
| 16 | % Shaders Busy |
| 18 | % Shader ALU Capacity Utilized |
| 22 | % Wave Context Occupancy |

**Where the stalls come from.** Split L1 from L2: a high L1 miss with a
negligible L2 miss is cache *thrash* from a scattered access pattern, not
bandwidth starvation, and the fix is the access pattern rather than smaller
textures:

| ID | Counter |
| --- | --- |
| 7 | % Texture Fetch Stall |
| 8 | % Texture L1 Miss |
| 9 | % Texture L2 Miss |
| 6 | % Vertex Fetch Stall |
| 21 | % Texture Pipes Busy |

**Work volume**, for comparing against Meta's budgets and deriving overdraw:

| ID | Counter |
| --- | --- |
| 25 | Vertices Shaded / Second |
| 32 | Fragments Shaded / Second |
| 27 | Textures / Vertex |
| 38 | Textures / Fragment |

### Deriving the numbers that matter

The counters are per-second; divide by measured FPS for per-frame figures:

```
vertices/frame  = (25) / fps
fragments/frame = (32) / fps
overdraw        = fragments/frame / (2 × eye_width × eye_height)
stage ms        = App_ms × (24 or 31) / 100
```

Meta's guidance is roughly **1–2 M vertices/frame for the whole app** and a
13.8 ms budget at 72 Hz. Overdraw meaningfully above ~1.5× on terrain-like
content is worth investigating on its own.

### Render stage trace

For per-stage timing rather than counters:

```sh
adb -s $D shell ovrgpuprofiler -e            # enable detailed mode FIRST
# relaunch the app — detailed mode only attaches to apps started after it
adb -s $D shell ovrgpuprofiler -t 1.0        # trace, seconds
adb -s $D shell ovrgpuprofiler -d            # disable when done
```

`-e` only affects applications started *after* it, so enable, then relaunch, or
the trace comes back empty. `-x` traces draw calls instead.

## Interpreting

Read the counters together, not individually. Some combinations that mean
something specific:

- **Shaders busy high + ALU utilization low + texture fetch stall high** →
  memory-latency bound. Reduce *fetch count*, not instruction count. Candidates:
  `textureGather` (four `texelFetch` → one instruction), baked lookup textures
  replacing computed values, texture arrays, sampling fewer layers.
- **L1 miss high + L2 miss ~0** → the working set fits in L2 but the access
  pattern thrashes L1. Improve locality; a smaller texture will not help.
- **Fragment share dominant** → foveated rendering, fewer fetches per fragment,
  and a distance fade that actually branches out of work.
- **Vertex share meaningful with vertex *fetch* stall low** → the cost is texture
  fetches inside the vertex shader, not attribute bandwidth. On Adreno, VS
  texture fetches run **twice** — once in the position-only binning shader, then
  again in the vertex shader — so they are doubly expensive.
- **Anisotropy is charged per fragment at grazing angles**, which is most of a
  terrain. It does not show up as a distinct counter; it shows up as texture
  fetch stall and texture pipes busy. Suspect it whenever tiled ground material
  is expensive, and test it by dropping taps to 1 — the cheapest possible A/B.

## Reporting

A counter dump is not a result. Turn it into a **web-viewable report** and share
the URL, so the reasoning survives past the terminal scrollback.

Write an HTML page and publish it with the **Artifact tool** (load the
`artifact-design` skill first). It renders as a private page on claude.ai that
can be shared, and it handles images as data URIs — which matters because device
captures are the other half of the evidence.

A good report contains, in order:

1. **The headline** — before/after frame time and FPS, and whether the budget is
   met. Lead with the number someone will act on.
2. **The stage split** — vertex vs fragment as milliseconds, not just percent.
3. **The bottleneck**, named, with the counters that identify it.
4. **Derived work volume** — vertices/frame and overdraw against Meta's budgets.
5. **What was measured vs inferred.** Label every estimate. A profiling report
   that blurs the two is worse than no report.
6. **Device captures** (`POST /capture`) for anything with a visual cost.
7. **Ranked next steps**, each tied to the counter that justifies it.

Include negative results. "Removing the grass saved 0.3 ms" and "the early-out
measured flat" are findings — they stop the next person spending a day on them.

## Finish cleanly

```sh
adb -s $D shell ovrgpuprofiler -d                                          # if -e was used
adb -s $D shell am broadcast -a com.oculus.vrpowermanager.automation_disable
```
