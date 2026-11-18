#!/usr/bin/env python3
"""Regenera os packs de áudio do shared-assets a partir dos manifests.

Os manifests (manifests/audio-*.yaml) são a fonte única de verdade: este
script lê-os e mapeia cada linha para o CLI ``text2sound`` (perfil, categoria,
duração, seed) — o mesmo contrato que o ``gameassets resume`` usa. Diferenças:
prompts curados sem injeção de mood, ``--no-enhance`` por omissão, tier
``highest`` (24 steps) e, para BGM, um **gate de costura** que re-rola a seed
(+1000, até 6 tentativas) até a cauda do loop ficar ≥70% do corpo (o outro
musical varia por geração — ver
docs/findings/TEXT2SOUND_SA3_LOOP_FINDINGS.md).

One-shots pedem -16 LUFS / -1 dBTP. Loops de campo no checkpoint SFX
pedem -19 LUFS / -3 dBTP, e os loops musicais usam o mastering do tier.

Uso (a partir de Text2Sound/, que tem o venv com a tool):
    .venv/bin/python /path/to/regen_audio.py            # só o que falta em disco
    .venv/bin/python /path/to/regen_audio.py --force    # regenera tudo
    .venv/bin/python /path/to/regen_audio.py --only bgm/boss,world/door_open
    .venv/bin/python /path/to/regen_audio.py --quality high --enhance
"""

from __future__ import annotations

import json
import subprocess
import sys
import time
from pathlib import Path
from tempfile import TemporaryDirectory

import yaml

POOL = Path(__file__).resolve().parent
MANIFESTS_DIR = POOL / "manifests"
AUDIO_ROOT = POOL / "public/assets/audio"
# Vael/assets → GitClones/AiGameKit/Text2Sound
T2S = [str(POOL.parent.parent / "AiGameKit/Text2Sound/.venv/bin/python"), "-m", "text2sound"]


def _probe_duration(path: Path) -> float | None:
    try:
        r = subprocess.run(
            ["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1", str(path)],
            capture_output=True,
            text=True,
            timeout=30,
        )
        return float(r.stdout.strip()) if r.returncode == 0 else None
    except (OSError, ValueError):
        return None


def _tail_head_pct(path: Path) -> tuple[float, float] | None:
    """(RMS cauda 500ms, RMS início 500ms) em % da mediana do corpo."""
    try:
        import numpy as np
        import soundfile as sf

        data, sr = sf.read(str(path), always_2d=True)
        # Preserve channel energy for mono recordings and wide stereo beds.
        energy = np.mean(data**2, axis=1)
        w = int(sr * 0.1)
        prof = np.sqrt(energy[: len(energy) // w * w].reshape(-1, w).mean(axis=1))
        body = float(np.median(prof))
        if body <= 0:
            return None
        return float(prof[-5:].mean() / body * 100), float(prof[:5].mean() / body * 100)
    except Exception:
        return None


# Categoria → audio_kind do QualityEngine (Shared/.../asset-categories.yaml).
# Duplicado só para carimbar o .json; a geração resolve pelo CLI.
_KIND_BY_CATEGORY = {
    "humanoid": "music_loop",
    "environment": "ambient_loop",
    "weapon": "sfx_impact",
    "prop": "sfx_interact",
    "chest": "sfx_interact",
    "door": "sfx_interact",
    "creature": "sfx_creature",
    "effects": "sfx_magic",
    "ui": "sfx_ui",
    "item": "sfx_ui",
    "movement": "sfx_movement",
    "collectible": "sfx_collectible",
    "vocal": "sfx_vocal",
    "vehicle": "sfx_vehicle",
}


def _check_signal(path: Path) -> None:
    """Reject empty, non-finite or effectively silent decoded candidates."""
    import numpy as np
    import soundfile as sf

    data, _ = sf.read(path, dtype="float32", always_2d=True)
    if not data.size or not np.isfinite(data).all() or np.max(np.abs(data)) < 10 ** (-55 / 20):
        raise ValueError("sinal vazio, inválido ou abaixo de -55 dBFS")


def _shape_tone(path: Path, lowpass_hz: int, makeup_db: float) -> None:
    """Apply the authored four-pole treble roll-off before publishing the clip."""
    filtered = path.with_suffix(".filtered.ogg")
    subprocess.run(
        [
            "ffmpeg",
            "-v",
            "error",
            "-y",
            "-i",
            str(path),
            "-af",
            f"lowpass=f={lowpass_hz},lowpass=f={lowpass_hz},volume={makeup_db}dB",
            "-c:a",
            "libvorbis",
            "-q:a",
            "8.5",
            str(filtered),
        ],
        check=True,
    )
    filtered.replace(path)


def _stamp_sidecar(out: Path, row: dict, quality: str, seed: int | None) -> None:
    """Repõe proveniência no .json ao lado do áudio.

    O caminho vramd do Text2Sound grava só prompt/duração/modelo; o in-process
    grava steps/quality/category/seed. Carimbar aqui mantém o pool auditável
    sem depender do worker (e sem re-gerar).
    """
    p = Path(str(out) + ".json")
    data = json.loads(p.read_text(encoding="utf-8"))
    data.setdefault("profile", row["profile"])
    data.setdefault("quality", quality)
    if quality == "highest":
        data.setdefault("steps", 24)
    elif quality == "high":
        data.setdefault("steps", 16)
    if row.get("category"):
        data.setdefault("category", row["category"])
        kind = _KIND_BY_CATEGORY.get(row["category"])
        if kind:
            data.setdefault("audio_kind", kind)
    data.setdefault("manifest", row["manifest"])
    if row.get("lowpass_hz"):
        data["lowpass_hz"] = row["lowpass_hz"]
        data["makeup_db"] = row["makeup_db"]
    if seed:
        data.setdefault("seed_effective", seed)
    p.write_text(json.dumps(data, indent=2, ensure_ascii=False), encoding="utf-8")


def load_rows() -> list[dict]:
    rows: list[dict] = []
    for mf in sorted(MANIFESTS_DIR.glob("audio-*.yaml")):
        data = yaml.safe_load(mf.read_text(encoding="utf-8"))
        for asset in data.get("assets", []):
            audio = asset.get("audio") or {}
            profile = audio.get("profile", "effects")
            duration = float(audio.get("duration", 1.0))
            # O modelo SFX recusa <0.5 s (e >30 s); o music aceita até 120 s.
            if profile == "effects":
                duration = min(max(duration, 0.5), 30.0)
            else:
                duration = min(max(duration, 0.5), 120.0)
            rows.append(
                {
                    "manifest": mf.stem,
                    "id": asset["id"],
                    "idea": asset["idea"].strip(),
                    "category": (asset.get("category") or "").strip() or None,
                    "profile": profile,
                    "duration": duration,
                    "field_loop": bool(audio.get("seamless_loop", False)) and profile == "effects",
                    "lowpass_hz": audio.get("lowpass_hz"),
                    "makeup_db": float(audio.get("makeup_db", 0)),
                    "seed": int(asset.get("seed", 0)),
                    # Legado de outros exemplos (racer): fica no pool mas não
                    # entra na regeneração por omissão.
                    "legacy": bool(asset.get("legacy", False)),
                }
            )
    return rows


def main() -> int:
    AUDIO_ROOT.mkdir(parents=True, exist_ok=True)
    with TemporaryDirectory(dir=AUDIO_ROOT) as work:
        return generate(Path(work))


def generate(work: Path) -> int:
    force = "--force" in sys.argv
    quality = "highest"
    if "--quality" in sys.argv:
        quality = sys.argv[sys.argv.index("--quality") + 1]
        if quality not in ("fast", "low", "medium", "high", "highest"):
            print(f"qualidade inválida: {quality}", file=sys.stderr)
            return 2
    enhance = "--enhance" in sys.argv
    include_legacy = "--include-legacy" in sys.argv
    only: set[str] | None = None
    if "--only" in sys.argv:
        # aceita ids exatos, sufixos (hit) ou prefixos de pasta (sfx, bgm, sfx/ui)
        only = set(sys.argv[sys.argv.index("--only") + 1].split(","))

    rows = load_rows()
    if only is not None:

        def matches(rid: str) -> bool:
            tail = rid.split("/")[-1]
            return any(rid == o or tail == o or rid.startswith(o + "/") for o in only)

        rows = [r for r in rows if matches(r["id"])]
    if not include_legacy:
        rows = [r for r in rows if not r["legacy"]]

    ok, skip, fail = 0, 0, 0
    total = len(rows)
    start_all = time.time()

    for idx, row in enumerate(rows, 1):
        out = AUDIO_ROOT / f"{row['id']}.ogg"
        cat_label = row["category"] or "-"
        print(f"\n[{idx}/{total}] {row['id']} ({row['profile']}/{cat_label}, {row['duration']}s, seed {row['seed']})")

        if out.exists() and not force:
            skip += 1
            print(f"  = já existe ({_probe_duration(out) or '?'}s) — skip (--force para regenerar)")
            continue

        out.parent.mkdir(parents=True, exist_ok=True)
        candidate = work / f"{row['id']}.ogg"
        candidate.parent.mkdir(parents=True, exist_ok=True)
        ambient = row["category"] == "environment"
        loop = row["field_loop"] or row["profile"] == "music"
        cmd = [
            *T2S,
            "generate",
            row["idea"],
            "--profile",
            row["profile"],
            "--quality",
            quality,
            "--vramd-priority",
            "batch",
            "--duration",
            str(row["duration"]),
            "-f",
            "ogg",
            "-o",
            str(candidate),
        ]
        if not enhance:
            # Prompts são autorais (Coro de Vael): o enhancer genérico aterra
            # "driving rhythm" em temas lentos e dilui a direção.
            cmd.append("--no-enhance")
        if row["field_loop"]:
            # Select the field-effects checkpoint explicitly: environment otherwise selects Music.
            cmd.extend(
                [
                    "--model",
                    "sfx",
                    "--seamless-loop",
                    "--crossfade-ms",
                    "800",
                    "--loop-edge-trim",
                    "1",
                    "--lufs",
                    "-19",
                    "--true-peak",
                    "-3",
                ]
            )
        elif row["profile"] == "effects":
            # One-shots: -14 LUFS empilhava picos em combate. Música/ambiência
            # ficam no mastering do tier.
            cmd.extend(["--lufs", "-16", "--true-peak", "-1.0"])
        if row["category"]:
            cmd.extend(["--category", row["category"]])

        # Ambiência é mais errática no outro; 5 seeds custam ~2 min e evitam
        # um loop com fade audível. BGM em 6 (17 s cada): o deserto/pântano
        # fecham com cauda curta em várias seeds.
        attempts = 5 if ambient else (6 if row["profile"] == "music" else 1)
        # Ambiência não tem cadência musical: o outro do SA3 é mais curto e a
        # cauda pode ser naturalmente esparsa (água, chuva). 70% é o gate de
        # MÚSICA; para `environment`/ambient_loop 30% já evita o fade audível.
        # O teto do INÍCIO (250%) evita o "blast" de intro a repetir a cada
        # loop (pássaro/gusto no primeiro meio segundo).
        if ambient:
            min_tail, min_head, max_head = 30.0, 50.0, 250.0
        else:
            min_tail, min_head, max_head = 70.0, 60.0, 250.0
        gen_seed = row["seed"]
        accepted = False
        result = None
        elapsed = 0.0
        for _attempt in range(attempts):
            cmd_seeded = [*cmd]
            if gen_seed:
                cmd_seeded.extend(["--seed", str(gen_seed)])
            t0 = time.time()
            result = subprocess.run(cmd_seeded, capture_output=True, text=True, timeout=900)
            elapsed = time.time() - t0
            if result.returncode != 0:
                break
            th = _tail_head_pct(candidate) if loop else None
            if not loop or (th and th[0] >= min_tail and min_head <= th[1] <= max_head):
                accepted = True
                break
            print(f"  ↻ seed {gen_seed}: cauda/início (% do corpo) = {th} — re-roll")
            gen_seed += 1000

        if accepted:
            dur = _probe_duration(candidate)
            if loop:
                # BGM seamless: contrato exacto (-d).
                good = dur is not None and abs(dur - row["duration"]) <= 0.25
            else:
                # SFX: o trim por kind acha o comprimento NATURAL do evento
                # (o -d é alvo/conditioning, não contrato — ver findings SA3).
                # Só rejeitar patologia grosseira (janelas não trimmadas).
                good = dur is not None and dur <= max(row["duration"] * 2.5, 4.5)
            if good:
                try:
                    if row["lowpass_hz"]:
                        _shape_tone(candidate, row["lowpass_hz"], row["makeup_db"])
                    _check_signal(candidate)
                    _stamp_sidecar(candidate, row, quality, gen_seed)
                except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
                    print(f"  ✗ acabamento: {error}")
                    fail += 1
                    continue
                candidate.replace(out)
                Path(str(candidate) + ".json").replace(Path(str(out) + ".json"))
                ok += 1
                print(f"  ✓ {out.stat().st_size} bytes in {elapsed:.1f}s ({dur:.2f}s)")
            else:
                fail += 1
                print(f"  ✗ duração fora do esperado: {dur}s (pedido {row['duration']}s)")
        elif result is not None and result.returncode != 0:
            fail += 1
            print(f"  ✗ FAILED ({elapsed:.1f}s)")
            # O CLI/rich mete o erro útil ANTES da caixa de ajuda; a cauda é
            # só o rodapé do vramd. Mostra as primeiras linhas úteis.
            err = (result.stderr or result.stdout or "").strip().splitlines()
            useful = [l for l in err if l.strip() and "│" not in l][:4]
            print("    " + "\n    ".join(useful))
        else:
            fail += 1
            th = _tail_head_pct(candidate)
            print(f"  ✗ gate de loop abaixo do limiar após {attempts} seeds ({th})")

    total_time = time.time() - start_all
    print(f"\n{'═' * 60}")
    print(f"Áudio: {ok} gerados, {skip} skips, {fail} falhas em {total_time:.0f}s")
    return 0 if fail == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
