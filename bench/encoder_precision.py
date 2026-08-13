#!/usr/bin/env python3
"""Time one encoder pass at fp32 and at int8, in both CPU runtimes.

    pip install onnxruntime openvino
    taskset -c 0-3 python bench/encoder_precision.py --fp32 fp32/tiny-encoder.onnx \
                                                     --int8 int8/tiny-encoder.int8.onnx

## Why this exists separately from `summo-bench asr`

`summo-bench asr` measures the thing users feel: audio in, text out. It found that int8 whisper-tiny
is exactly as fast as fp32 end to end, which is a surprising enough result to be worth explaining
rather than just recording — a quantised model is *supposed* to be faster.

This script isolates the encoder, which is the half of whisper that looks like the matrix
multiplication quantisation is good at: one pass over a fixed 30 s window, large matrices. It turns
out int8 wins there by about 1.4× in ONNX Runtime and loses the rest back in the autoregressive
decoder, whose matrices are small enough that the quantise/dequantise around each one costs more
than the cheaper multiply saves.

It runs OpenVINO alongside ONNX Runtime to answer the other suspicion — that the machine, not the
model, is bad at int8. A runtime tuned by Intel for Intel silicon disagreeing with ORT about *which*
precision is faster is the evidence that this is a property of the graph.

## Why `taskset`

Without pinning, a thread pool asked for four threads on an idle 64-core machine does not behave
like four cores on a laptop: the OS spreads it across sockets and the memory-bound parts get more
bandwidth than any real machine would give them. Pinning to four hardware threads on one socket is
the closest this hardware gets to the machine the answer is for.
"""

from __future__ import annotations

import argparse
import statistics
import time

import numpy as np


def bench_ort(path: str, mel: np.ndarray, threads: int, runs: int) -> list[float]:
    import onnxruntime as ort

    options = ort.SessionOptions()
    options.intra_op_num_threads = threads
    # The default already, set explicitly so the comparison cannot be blamed on it.
    options.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
    session = ort.InferenceSession(path, options, providers=["CPUExecutionProvider"])
    feed = {session.get_inputs()[0].name: mel}

    for _ in range(3):  # warm up: first passes allocate arenas and pick kernels
        session.run(None, feed)
    timings = []
    for _ in range(runs):
        start = time.perf_counter()
        session.run(None, feed)
        timings.append(time.perf_counter() - start)
    return timings


def bench_openvino(path: str, mel: np.ndarray, threads: int, runs: int) -> list[float]:
    import openvino as ov

    core = ov.Core()
    model = core.read_model(path)
    # The exported graph has a dynamic batch and frame count; a fixed shape is what a runtime can
    # actually optimise for, and it is what the app feeds.
    model.reshape({model.inputs[0].get_any_name(): ov.PartialShape(list(mel.shape))})
    compiled = core.compile_model(
        model, "CPU", {"INFERENCE_NUM_THREADS": threads, "PERFORMANCE_HINT": "LATENCY"}
    )
    request = compiled.create_infer_request()
    feed = {compiled.inputs[0].get_any_name(): mel}

    for _ in range(3):
        request.infer(feed)
    timings = []
    for _ in range(runs):
        start = time.perf_counter()
        request.infer(feed)
        timings.append(time.perf_counter() - start)
    return timings


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--fp32", required=True, help="full-precision encoder .onnx")
    parser.add_argument("--int8", required=True, help="quantised encoder .onnx")
    parser.add_argument("--threads", type=int, default=4)
    parser.add_argument("--runs", type=int, default=15)
    parser.add_argument(
        "--frames",
        type=int,
        default=3000,
        help="mel frames; 3000 is whisper's fixed 30 s window",
    )
    args = parser.parse_args()

    # Random mel rather than real audio: the encoder does the same work whatever the values are, and
    # a fixed shape keeps the two precisions comparable.
    mel = np.random.randn(1, 80, args.frames).astype(np.float32)

    runners = [("onnxruntime", bench_ort)]
    try:
        import openvino  # noqa: F401

        runners.append(("openvino", bench_openvino))
    except ImportError:
        print("openvino not installed; reporting ONNX Runtime only\n")

    print(f"{'runtime':<14}{'precision':<10}{'median ms':>10}{'min ms':>9}")
    for name, run in runners:
        results = {}
        for precision, path in (("fp32", args.fp32), ("int8", args.int8)):
            timings = run(path, mel, args.threads, args.runs)
            results[precision] = statistics.median(timings)
            print(
                f"{name:<14}{precision:<10}"
                f"{statistics.median(timings) * 1000:10.1f}{min(timings) * 1000:9.1f}"
            )
        ratio = results["fp32"] / results["int8"]
        verdict = "faster" if ratio > 1 else "slower"
        print(f"{'':14}int8 is {abs(ratio if ratio > 1 else 1 / ratio):.2f}× {verdict}\n")


if __name__ == "__main__":
    main()
