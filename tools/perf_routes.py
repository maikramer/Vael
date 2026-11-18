"""Mede streaming, câmara parada e caminhada com o profiler do Vael.

Exemplo: python tools/perf_routes.py /tmp/rotas --binary target/release/vael
"""

from __future__ import annotations

import argparse
import json
import math
import os
import statistics
import subprocess
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from qa import ROOT, Bridge, live_step, screenshot

# XZ do jogador. A hora e o clima arrancam já no preset de cada sessão.
ROUTES = {
    "Blizzard": (15, [("picos", -28, -300), ("vila", 0, 44)]),
    "Rain": (21, [("pantano", -276, -42), ("floresta", -42, 120)]),
    "Clear": (9, [("deserto", 122, 98), ("ermo", 1518, -724)]),
}


def window(bridge):
    return bridge.rpc("viber.profiler.tab", {"tab": "frames"})["history"]


def measure(bridge, previous, seconds):
    """Une as janelas sobrepostas do ring, incluindo os frames lentos da entrada."""
    samples = []
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        # Ler antes de a janela de 240 frames dar uma volta, também em ecrãs rápidos.
        interval = min(1.0, max(0.05, sum(previous) / 2000.0))
        time.sleep(min(interval, max(0, deadline - time.monotonic())))
        snapshot = bridge.rpc("viber.profiler.tab", {"tab": "frames"})
        current = snapshot["history"]
        overlap = min(len(previous), len(current))
        while overlap and previous[-overlap:] != current[:overlap]:
            overlap -= 1
        if not overlap:
            raise RuntimeError("Amostras descontínuas do profiler")
        samples.extend(current[overlap:])
        previous = current
    if not samples:
        raise RuntimeError("Nenhum frame novo durante a medição")
    ranked = sorted(samples)

    def percentile(p):
        return ranked[math.ceil(p * len(ranked)) - 1]

    report = {
        "fps": 1000 / statistics.mean(samples),
        "p95_ms": percentile(0.95),
        "p99_ms": percentile(0.99),
        "over_50ms": sum(t > 50 for t in samples),
        "frames_ms": samples,
        "top": snapshot["top"],
        "hitches": snapshot["hitches"],
    }
    if os.environ.get("VIBER_PROF_GPU") == "1":
        report["gpu"] = bridge.rpc("viber.profiler.tab", {"tab": "systems"})["gpu"]
    return report


def run_route(bridge, output, name, x, z):
    before = window(bridge)
    bridge.lua(
        f"local h=viber.debug.terrain({x},{z}).height; "
        f"viber.teleport_player({x},h+0.3,{z}); viber.camera_free(true); "
        f"local eye=viber.debug.terrain({x + 4},{z + 6}).height; "
        f"local target=viber.debug.terrain({x},{z - 8}).height; "
        f"viber.camera_pose({x + 4},eye+5,{z + 6},{x},target+2,{z - 8})"
    )
    arrival = measure(bridge, before, 10)
    time.sleep(5)
    fixed = measure(bridge, window(bridge), 12)
    if not screenshot(bridge.port, output / f"{name}.png"):
        raise RuntimeError(f"Falha na captura de {name}")
    climate = bridge.lua("return viber.vael.env_info()")
    bridge.lua("viber.camera_free(false)")
    time.sleep(2)
    start = bridge.lua("return viber.debug.player()")
    before = window(bridge)
    bridge.call("viber.input.key", {"key": "w", "state": "press"})
    try:
        walking = measure(bridge, before, 12)
    finally:
        bridge.call("viber.input.key", {"key": "w", "state": "release"})
    end = bridge.lua("return viber.debug.player()")
    distance = math.hypot(end["x"] - start["x"], end["z"] - start["z"])
    report = dict(
        arrival=arrival,
        fixed=fixed,
        walking=walking,
        start=start,
        end=end,
        displacement_m=distance,
        climate=climate,
    )
    (output / f"{name}.json").write_text(json.dumps(report))
    print(
        name,
        round(distance, 1),
        "m",
        [
            (round(r["fps"], 1), round(r["p99_ms"], 1))
            for r in [arrival, fixed, walking]
        ],
        flush=True,
    )


TOURS = ("arena_combat", "forge_interior")


def run_tour(bridge, output, name, steps):
    def play():
        for step in steps:
            live_step(bridge, step, output)

    # O mesmo tour de QA alimenta os comandos enquanto o profiler lê os frames.
    previous = window(bridge)
    with ThreadPoolExecutor(max_workers=1) as worker:
        actions = worker.submit(play)
        duration = sum(step.get("wait", 0) for step in steps) + 3
        active = measure(bridge, previous, duration)
        actions.result()
    if not screenshot(bridge.port, output / f"{name}.png"):
        raise RuntimeError(f"Falha na captura de {name}")
    report = {"active": active, "steps": steps}
    (output / f"{name}.json").write_text(json.dumps(report))
    print(name, round(active["fps"], 1), round(active["p99_ms"], 1), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out", type=Path)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/vael")
    parser.add_argument("--port", type=int, default=15888)
    selection = parser.add_mutually_exclusive_group()
    selection.add_argument("--weather", nargs="+", choices=ROUTES)
    selection.add_argument("--tour", nargs="+", choices=TOURS)
    args = parser.parse_args()
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=True)
    bridge = Bridge(args.port)
    if bridge.listening():
        parser.error(f"Porta {args.port} já ocupada")
    for case in args.weather or args.tour or [*ROUTES, *TOURS]:
        if case in ROUTES:
            hour, places = ROUTES[case]
            launch = ["world.bsn", "--scene", "main", "--entry", "dev"]
            settings = {
                "VAEL_START_HOUR": str(hour),
                "VAEL_START_WEATHER": case,
                "VAEL_START_SNOW_DEPTH": "0.30" if case == "Blizzard" else "0",
            }
        else:
            tour = json.loads((ROOT / "qa/vision/tours" / f"{case}.json").read_text())
            launch = ["--scenario", tour["qa_scenario"]]
            settings = {}
        with (
            tempfile.TemporaryDirectory() as save,
            (output / f"{case}.log").open("w") as log,
        ):
            process = subprocess.Popen(
                [
                    args.binary.resolve(),
                    *launch,
                    "--bridge",
                    str(args.port),
                    "--resolution",
                    "1280x720",
                ],
                cwd=ROOT,
                env=dict(os.environ, VAEL_QA_SAVE_DIR=save, **settings),
                stdout=log,
                stderr=log,
            )
            try:
                if not bridge.wait_ready(150) or process.poll() is not None:
                    raise RuntimeError(f"Arranque falhou. Ver {log.name}")
                if case in ROUTES:
                    for name, x, z in places:
                        run_route(bridge, output, name, x, z)
                else:
                    run_tour(bridge, output, case, tour["steps"])
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=20)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()


if __name__ == "__main__":
    main()
