#!/usr/bin/env python3
"""tools/avian_baseline.py — fixtures de paridade da física (Rapier → Avian).

Antes de trocar o motor, captura o que o jogo actual responde; depois da
troca, o `compare` diz exactamente o que mudou. O oráculo é o bridge: os
mesmos endpoints (`viber.raycast`, `vael.debug.colliders`,
`vael.debug.player/stats`) têm de continuar a responder no build novo.

  avian_baseline.py capture --port 15740 [--out docs/qa/avian] [--perf]
  avian_baseline.py hero    --port 15740 [--out docs/qa/avian]
  avian_baseline.py compare --before DIR_A --after DIR_B

Fixtures:
  meta.json       commit, hash do `world.bsn`, binário, versões dos motores
  rays.json       grelha fixa de 314 raios (toi, normal, nome do alvo)
  colliders.json  props nomeados junto a 3 âncoras (forma + half-extents)
  hero.json       invariantes de movimento (andar, saltar, repouso)
  perf.jsonl      `vael-tools debug prof --record` (opcional)

O jogo tem de estar a correr com `--bridge` e o mundo carregado
(`vael-tools debug --port P wait-ready`). Uma fixture só é comparável com o mesmo
`world.bsn`: o `meta.json` guarda o hash e o `compare` recusa hashes
diferentes.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
from qa import Bridge, short, tools_bin  # noqa: E402  (cliente JSON-RPC e CLI do bridge)

DEFAULT_OUT = ROOT / "docs" / "qa" / "avian"
GRID_STEP = 4.0
GRID_RADIUS = 32.0
RAY_TOL = 0.01  # metros
EXTENT_TOL = 0.01


# ── utilidades ──────────────────────────────────────────────────────────


def git(*args: str) -> str:
    try:
        return subprocess.run(
            ["git", *args], cwd=ROOT, capture_output=True, text=True, timeout=20
        ).stdout.strip()
    except Exception:
        return ""


def world_hash() -> str:
    path = ROOT / "world.bsn"
    return hashlib.sha256(path.read_bytes()).hexdigest()[:16] if path.exists() else ""


def engine_versions() -> dict[str, str]:
    """Versões do lock que interessam à física (nome → versão)."""
    out: dict[str, str] = {}
    lock = ROOT / "Cargo.lock"
    if not lock.exists():
        return out
    text = lock.read_text()
    for name in ("bevy_rapier3d", "rapier3d", "parry3d", "avian3d",
                 "bevy-tnua", "bevy-tnua-avian3d"):
        marker = f'name = "{name}"\nversion = "'
        at = text.find(marker)
        if at >= 0:
            rest = text[at + len(marker):]
            out[name] = rest.split('"', 1)[0]
    return out


def binary_info() -> dict[str, object]:
    path = ROOT / "target" / "release" / "vael"
    if not path.exists():
        return {"missing": True}
    st = path.stat()
    return {"bytes": st.st_size, "mtime": int(st.st_mtime)}


def player(bridge: Bridge) -> dict | None:
    res = bridge.lua("return vael.debug.player()")
    return res if isinstance(res, dict) else None


def stats(bridge: Bridge) -> dict:
    res = bridge.lua("return vael.debug.stats()")
    return res if isinstance(res, dict) else {}


def teleport(bridge: Bridge, pos: list[float]) -> None:
    bridge.lua(f"return vael.debug.teleport({pos[0]}, {pos[1]}, {pos[2]})")
    time.sleep(0.25)


def settle(bridge: Bridge, secs: float = 0.5) -> None:
    time.sleep(secs)


# ── capture ─────────────────────────────────────────────────────────────


def ray_fixture(origin: list[float]) -> list[dict]:
    """Grelha fixa em torno de `origin`: 289 verticais + 17 horizontais + 8 radiais."""
    ox, oy, oz = origin
    rays: list[dict] = []
    steps = int(GRID_RADIUS / GRID_STEP)
    for i in range(-steps, steps + 1):
        for j in range(-steps, steps + 1):
            rays.append({
                "kind": "down",
                "o": [ox + i * GRID_STEP, oy + 40.0, oz + j * GRID_STEP],
                "d": [0.0, -1.0, 0.0],
                "max_toi": 120.0,
            })
    for i in range(-steps, steps + 1):
        rays.append({
            "kind": "east",
            "o": [ox - 40.0, oy + 1.0, oz + i * GRID_STEP],
            "d": [1.0, 0.0, 0.0],
            "max_toi": 80.0,
        })
    for k in range(8):
        import math

        ang = k * math.pi / 4.0
        rays.append({
            "kind": "radial",
            "o": [ox, oy + 1.5, oz],
            "d": [math.cos(ang), 0.0, math.sin(ang)],
            "max_toi": 60.0,
        })
    return rays


def cast_all(bridge: Bridge, rays: list[dict]) -> list[dict]:
    out: list[dict] = []
    for ray in rays:
        params = {
            "x": ray["o"][0], "y": ray["o"][1], "z": ray["o"][2],
            "dx": ray["d"][0], "dy": ray["d"][1], "dz": ray["d"][2],
            "max_toi": ray["max_toi"],
        }
        try:
            res = bridge.call("viber.raycast", params)
        except Exception as err:  # noqa: BLE001 — guardar o erro é o resultado
            out.append({"kind": ray["kind"], "error": str(err)[:200]})
            continue
        out.append({
            "kind": ray["kind"],
            "hit": bool(res.get("hit")),
            "toi": round(float(res.get("toi", -1.0)), 5) if res.get("hit") else None,
            "name": res.get("root_name") or res.get("name"),
            "normal": [round(v, 4) for v in res.get("normal", [])]
            if res.get("hit") else None,
        })
    return out


def collider_fixture(bridge: Bridge, origin: list[float]) -> list[dict]:
    """Props nomeados à volta de 3 âncoras (o dump é limitado a 256 e ao raio)."""
    anchors = [(0.0, 0.0), (12.0, 0.0), (0.0, 12.0)]
    picked: dict[str, dict] = {}
    for dx, dz in anchors:
        if (dx, dz) != (0.0, 0.0):
            bridge.lua(f"return vael.debug.move_player({dx}, {dz})")
            time.sleep(0.35)
        rows = bridge.lua("return vael.debug.colliders(8)") or []
        for row in rows if isinstance(rows, list) else []:
            if not isinstance(row, dict):
                continue
            name = row.get("name")
            if not name:
                continue
            # Offsets de âncora ficam no nome para as âncoras não colidirem.
            picked.setdefault(
                name,
                {
                    "name": name,
                    "shape": row.get("shape"),
                    "hx": row.get("hx"),
                    "hy": row.get("hy"),
                    "hz": row.get("hz"),
                    "radius": row.get("radius"),
                    "rigidbody": row.get("rigidbody"),
                    "x": round(float(row.get("x", 0.0)), 3),
                    "y": round(float(row.get("y", 0.0)), 3),
                    "z": round(float(row.get("z", 0.0)), 3),
                },
            )
    # Volta à origem.
    bridge.lua(f"return vael.debug.move_player({-12.0}, {0.0})")
    bridge.lua(f"return vael.debug.move_player({0.0}, {-12.0})")
    return sorted(picked.values(), key=lambda item: (item["name"], item["x"], item["z"]))


def cmd_capture(args) -> None:
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    bridge = Bridge(args.port)
    if not bridge.wait_ready(args.timeout):
        sys.exit(f"avian_baseline: o jogo não ficou ready em {args.timeout:.0f}s")

    who = player(bridge)
    if not who:
        sys.exit("avian_baseline: sem player — o mundo não carregou?")
    origin = [round(float(who["x"]), 1), round(float(who["y"]), 1), round(float(who["z"]), 1)]

    meta = {
        "captured_at": int(time.time()),
        "commit": git("rev-parse", "HEAD"),
        "dirty": len([l for l in git("status", "--porcelain").splitlines() if l]),
        "world_sha256_16": world_hash(),
        "engines": engine_versions(),
        "binary": binary_info(),
        "player_origin": origin,
    }
    (out / "meta.json").write_text(json.dumps(meta, indent=2) + "\n")

    rays = ray_fixture(origin)
    results = cast_all(bridge, rays)
    hits = sum(1 for r in results if r.get("hit"))
    errors = sum(1 for r in results if r.get("error"))
    (out / "rays.json").write_text(json.dumps(results, indent=1) + "\n")
    print(f"rays: {len(results)} ({hits} hit, {errors} erro) → {out / 'rays.json'}")
    if errors:
        print(
            f"avian_baseline: {errors}/{len(results)} raios sem resultado — "
            f"a fixture {out / 'rays.json'} não serve para comparar",
            file=sys.stderr,
        )
        # O motivo importa: sem `viber.raycast` no bridge não há fixture de
        # raios nenhuma (o `vael-tools/raycast` do protocolo ainda não tem
        # handler no runtime).
        reasons = sorted({(r.get("error") or "").strip()[:160] for r in results if r.get("error")})
        for reason in reasons[:3]:
            print(f"  - {reason}", file=sys.stderr)

    colliders = collider_fixture(bridge, origin)
    (out / "colliders.json").write_text(json.dumps(colliders, indent=1) + "\n")
    print(f"colliders: {len(colliders)} nomeados → {out / 'colliders.json'}")

    if args.perf:
        perf = out / "perf.jsonl"
        try:
            res = subprocess.run(
                [*tools_bin(), "debug", "--port", str(args.port), "prof",
                 "--record", "10", "--hz", "2", "-o", str(perf)],
                capture_output=True, text=True, timeout=60,
            )
            if res.returncode != 0:
                print(
                    f"perf: falhou (exit {res.returncode}) "
                    f"{short(res.stderr.strip())}",
                    file=sys.stderr,
                )
            else:
                print(f"perf: {perf}")
        except Exception as err:  # noqa: BLE001
            print(f"perf: falhou ({err})", file=sys.stderr)

    if errors:
        if args.allow_missing:
            print(
                "avian_baseline: --allow-missing: rays.json fica com os erros por "
                "raio (o bridge não serve `viber.raycast`); meta/colliders/hero "
                "continuam válidos, mas sem raios o `compare` não prova paridade",
                file=sys.stderr,
            )
            return
        # Exit 1: uma captura sem raios não é uma baseline válida.
        sys.exit(1)


# ── hero ────────────────────────────────────────────────────────────────


def sample(bridge: Bridge, seconds: float, hz: float = 10.0) -> list[dict]:
    out: list[dict] = []
    end = time.time() + seconds
    while time.time() < end:
        who = player(bridge)
        st = stats(bridge)
        if who:
            out.append({
                "x": round(float(who["x"]), 3),
                "y": round(float(who["y"]), 3),
                "z": round(float(who["z"]), 3),
                "grounded": bool(st.get("player_grounded")),
                "vel_y": round(float(st.get("player_vel_y", 0.0)), 3),
            })
        time.sleep(1.0 / hz)
    return out


def hold_key(port: int, key: str, hold_ms: int) -> subprocess.Popen:
    return subprocess.Popen(
        [*tools_bin(), "debug", "--port", str(port), "key", key, "--hold-ms", str(hold_ms)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def cmd_hero(args) -> None:
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    bridge = Bridge(args.port)
    if not bridge.wait_ready(args.timeout):
        sys.exit(f"avian_baseline: o jogo não ficou ready em {args.timeout:.0f}s")
    who = player(bridge)
    if not who:
        sys.exit("avian_baseline: sem player")
    anchor = [float(who["x"]), float(who["y"]), float(who["z"])]

    # Andar: 2 s com W. Amostra enquanto a tecla está premida.
    teleport(bridge, anchor)
    settle(bridge)
    before = player(bridge)
    proc = hold_key(args.port, "w", 2000)
    walk_samples = sample(bridge, 2.3)
    proc.wait(timeout=10)
    if proc.returncode != 0:
        raise RuntimeError(f"`debug key w --hold-ms 2000` falhou (exit {proc.returncode})")
    settle(bridge, 0.4)
    after = player(bridge)
    walk = {
        "from": [round(float(before[k]), 3) for k in "xyz"],
        "to": [round(float(after[k]), 3) for k in "xyz"],
        "dist": round(
            sum((float(after[k]) - float(before[k])) ** 2 for k in ("x", "z")) ** 0.5, 3
        ),
        "grounded": round(
            sum(1 for s in walk_samples if s["grounded"]) / max(1, len(walk_samples)), 3
        ),
        "samples": walk_samples[::5],
    }

    # Salto: tecla uma vez, amostra 1.2 s.
    teleport(bridge, anchor)
    settle(bridge, 0.6)
    ground_y = float(player(bridge)["y"])
    # Sem `check`: um `debug key` que falha deixava `peak` a 0 em silêncio e a
    # fixture parecia "o herói não salta" em vez de "a tecla não chegou".
    pressed = subprocess.run(
        [*tools_bin(), "debug", "--port", str(args.port), "key", "space"],
        capture_output=True, text=True, timeout=20,
    )
    if pressed.returncode != 0:
        raise RuntimeError(
            f"`debug key space` falhou (exit {pressed.returncode}): "
            f"{short(pressed.stderr.strip())}"
        )
    jump_samples = sample(bridge, 1.2)
    peak = max((s["y"] for s in jump_samples), default=ground_y)
    end_y = jump_samples[-1]["y"] if jump_samples else ground_y
    jump = {
        "ground_y": round(ground_y, 3),
        "peak": round(peak - ground_y, 3),
        "landed": round(end_y - ground_y, 3),
        "samples": jump_samples[::3],
    }

    # Repouso: 1.5 s parado; deriva e chão.
    teleport(bridge, anchor)
    settle(bridge, 0.6)
    idle_from = player(bridge)
    idle_samples = sample(bridge, 1.5)
    idle_to = player(bridge)
    idle = {
        "drift": round(
            sum((float(idle_to[k]) - float(idle_from[k])) ** 2 for k in ("x", "z")) ** 0.5, 4
        ),
        "grounded": round(
            sum(1 for s in idle_samples if s["grounded"]) / max(1, len(idle_samples)), 3
        ),
    }

    data = {"anchor": anchor, "walk": walk, "jump": jump, "idle": idle}
    (out / "hero.json").write_text(json.dumps(data, indent=2) + "\n")
    print(f"walk {walk['dist']} m (grounded {walk['grounded']}), "
          f"jump pico {jump['peak']} m, repouso deriva {idle['drift']} m "
          f"→ {out / 'hero.json'}")


# ── compare ─────────────────────────────────────────────────────────────


def reload(path: Path) -> object:
    return json.loads(path.read_text())


def cmd_compare(args) -> None:
    before, after = Path(args.before), Path(args.after)
    problems: list[str] = []

    mb, ma = reload(before / "meta.json"), reload(after / "meta.json")
    if mb.get("world_sha256_16") != ma.get("world_sha256_16"):
        problems.append(
            f"world.bsn mudou ({mb.get('world_sha256_16')} → {ma.get('world_sha256_16')}): "
            "as fixtures não são comparáveis"
        )
    ob, oa = mb.get("player_origin"), ma.get("player_origin")
    if ob and oa and (abs(ob[0] - oa[0]) > 0.5 or abs(ob[2] - oa[2]) > 0.5):
        problems.append(f"origem do grid do player mudou: {ob} → {oa}")

    rb, ra = reload(before / "rays.json"), reload(after / "rays.json")
    if len(rb) != len(ra):
        problems.append(f"rays: {len(rb)} vs {len(ra)} (mesma grelha?)")
    mismatched = 0
    worst = 0.0
    # `strict=False`: grelhas de tamanhos diferentes já foram reportadas acima
    # e o resto compara-se no prefixo comum.
    for i, (b, a) in enumerate(zip(rb, ra, strict=False)):
        # Um raio sem resultado (`error`) não é "sem hit": duas fixtures cheias
        # de erros comparavam iguais e davam "paridade OK" a mentir.
        if b.get("error") or a.get("error"):
            problems.append(
                f"ray {i} ({b.get('kind')}): sem resultado "
                f"({b.get('error') or a.get('error')})"
            )
            mismatched += 1
            continue
        if bool(b.get("hit")) != bool(a.get("hit")):
            problems.append(f"ray {i} ({b['kind']}): hit {b.get('hit')} → {a.get('hit')}")
            mismatched += 1
            continue
        if b.get("hit") and a.get("hit"):
            d = abs(float(b["toi"]) - float(a["toi"]))
            worst = max(worst, d)
            if d > RAY_TOL:
                problems.append(
                    f"ray {i} ({b['kind']}): toi {b['toi']} → {a['toi']} (Δ {d:.4f})"
                )
                mismatched += 1
            elif b.get("name") != a.get("name"):
                problems.append(
                    f"ray {i} ({b['kind']}): alvo {b.get('name')} → {a.get('name')}"
                )
    print(f"rays: {len(rb)} comparados, {mismatched} divergentes, Δtoi máx {worst:.5f} m")

    cb, ca = reload(before / "colliders.json"), reload(after / "colliders.json")
    by_b = {(c["name"], round(c["x"], 1), round(c["z"], 1)): c for c in cb}
    by_a = {(c["name"], round(c["x"], 1), round(c["z"], 1)): c for c in ca}
    missing = sorted(set(by_b) - set(by_a))
    extra = sorted(set(by_a) - set(by_b))
    if missing:
        problems.append(f"colliders em falta: {len(missing)} (ex.: {missing[:3]})")
    if extra:
        problems.append(f"colliders novos: {len(extra)} (ex.: {extra[:3]})")
    changed = 0
    for key in set(by_b) & set(by_a):
        b, a = by_b[key], by_a[key]
        if b.get("shape") != a.get("shape"):
            problems.append(f"{key[0]}: forma {b.get('shape')} → {a.get('shape')}")
            changed += 1
            continue
        for field in ("hx", "hy", "hz", "radius"):
            vb, va = b.get(field), a.get(field)
            if vb is None and va is None:
                continue
            if vb is None or va is None or abs(float(vb) - float(va)) > EXTENT_TOL:
                problems.append(f"{key[0]}: {field} {vb} → {va}")
                changed += 1
    print(f"colliders: {len(by_b)} comparados, {changed} com dimensão diferente")

    hb_path, ha_path = before / "hero.json", after / "hero.json"
    if hb_path.exists() and ha_path.exists():
        hb, ha = reload(hb_path), reload(ha_path)
        checks = [
            ("walk.dist", 0.5),
            ("walk.grounded", 0.10),
            ("jump.peak", 0.15),
            ("idle.drift", 0.05),
        ]
        for path, tol in checks:
            vb = hb
            va = ha
            for part in path.split("."):
                vb, va = vb.get(part), va.get(part)
            if vb is None or va is None or abs(float(vb) - float(va)) > tol:
                problems.append(f"{path}: {vb} → {va} (tol {tol})")
        print(
            f"herói: andar {hb['walk']['dist']}→{ha['walk']['dist']} m, "
            f"salto {hb['jump']['peak']}→{ha['jump']['peak']} m, "
            f"repouso {hb['idle']['drift']}→{ha['idle']['drift']} m"
        )

    if problems:
        print(f"\n{len(problems)} problema(s):", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        sys.exit(1)
    print("\nparidade OK")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="cmd", required=True)

    cap = sub.add_parser("capture", help="raios + colliders (+ perf) do jogo a correr")
    cap.add_argument("--port", type=int, default=15740)
    cap.add_argument("--out", default=str(DEFAULT_OUT))
    cap.add_argument("--timeout", type=float, default=180.0)
    cap.add_argument("--perf", action="store_true")
    cap.add_argument(
        "--allow-missing",
        action="store_true",
        help="não falha se o bridge não servir `viber.raycast` (fixture parcial)",
    )
    cap.set_defaults(func=cmd_capture)

    hero = sub.add_parser("hero", help="invariantes de movimento do herói")
    hero.add_argument("--port", type=int, default=15740)
    hero.add_argument("--out", default=str(DEFAULT_OUT))
    hero.add_argument("--timeout", type=float, default=180.0)
    hero.set_defaults(func=cmd_hero)

    cmp = sub.add_parser("compare", help="compara duas pastas de fixtures")
    cmp.add_argument("--before", required=True)
    cmp.add_argument("--after", required=True)
    cmp.set_defaults(func=cmd_compare)

    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
