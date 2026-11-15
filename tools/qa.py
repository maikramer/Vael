#!/usr/bin/env python3
"""tools/qa.py — QA do Vael: cenas leves, cenários de quest e execução.

Cenários (`qa/scenarios/*.json`, ver `src/qa.rs` e `docs/QA.md`):

  qa.py list                         cenários, cena e descrição
  qa.py new NOME --quest ID [...]    cenário de quest com passos e expect gerados
  qa.py sim [NOME|all]               simula sem janela (regras reais, ms)
  qa.py use NOME                     cenário do "Play (QA)" do Jackdaw (qa/.active)
  qa.py run NOME [--keep]            arranca o jogo, corre os passos vivos e verifica
  qa.py steps NOME --port P          reaplica e corre os passos num jogo aberto
  qa.py apply [NOME] --port P        reaplica num jogo a correr (hot)
  qa.py check [--port P]             verifica o `expect` no jogo a correr
  qa.py snapshot NOME --port P       grava o estado vivo como cenário novo
  qa.py state --port P               quests/itens/talentos do jogo a correr
  qa.py lua CODE --port P            Luau na VM dos scripts

Cenas QA (`scenes/qa/*.bsn`, chão plano, sem terreno — carregam em segundos):

  qa.py scene new NOME [--copy ENT[@x,z]]... [--enemy KIND:N]... [--interior]
  qa.py scene find TEXTO             entidades com nome no mundo (para --copy)
  qa.py scene quest ID               cena + cenário prontos para a quest ID

Sem dependências fora da stdlib. A bridge é o JSON-RPC do `--bridge`.
"""

from __future__ import annotations

import argparse
import json
import os
import random
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCEN_DIR = ROOT / "qa" / "scenarios"
SCENE_DIR = ROOT / "scenes" / "qa"
ACTIVE = ROOT / "qa" / ".active"
EXT = ROOT.parent / "bevy_extensions"
DEFAULT_PORT = 15740
WORLD_FILES = [ROOT / "world.bsn", *sorted((ROOT / "world").rglob("*.bsn"))]


# ── utilidades ──────────────────────────────────────────────────────────


def die(msg: str, code: int = 2) -> None:
    print(f"qa: {msg}", file=sys.stderr)
    sys.exit(code)


def load_quests() -> dict[str, dict]:
    out: dict[str, dict] = {}
    index = json.loads((ROOT / "quests" / "index.json").read_text())
    for name in index:
        for quest in json.loads((ROOT / "quests" / name).read_text()):
            out[quest["id"]] = quest
    return out


def stages(quest: dict) -> list[dict]:
    return quest.get("stages") or [quest.get("objective") or {}]


def scenario_path(name: str) -> Path:
    if name.endswith(".json"):
        p = Path(name)
        return p if p.is_absolute() else ROOT / p
    return SCEN_DIR / f"{name}.json"


def build_game(release: bool = True, log=None) -> Path:
    """`cargo build` do jogo ANTES do cronómetro do `ready`; devolve o binário."""
    cmd = ["cargo", "build", "--quiet", *(["--release"] if release else [])]
    print("qa: a compilar o jogo (cargo build)…", flush=True)
    if subprocess.call(cmd, cwd=ROOT, stdout=log, stderr=log) != 0:
        die("cargo build falhou" + (f"; ver {log.name}" if log else ""), 1)
    return ROOT / "target" / ("release" if release else "debug") / "vael"


def vael_cmd(*args: str, release: bool = True) -> list[str]:
    """`vael` compilado (cargo decide se recompila)."""
    cmd = ["cargo", "run", "--quiet"]
    if release:
        cmd.append("--release")
    return [*cmd, "--", *args]


# ── bridge ──────────────────────────────────────────────────────────────


class Bridge:
    def __init__(self, port: int, timeout: float = 30.0):
        self.port = port
        self.timeout = timeout

    def call(self, method: str, params: dict | None = None, timeout: float | None = None):
        body = json.dumps(
            {"jsonrpc": "2.0", "id": 0, "method": method, "params": params or {}}
        ).encode()
        req = urllib.request.Request(
            f"http://127.0.0.1:{self.port}/",
            data=body,
            headers={"Content-Type": "application/json"},
        )
        with urllib.request.urlopen(req, timeout=timeout or self.timeout) as resp:
            data = json.loads(resp.read())
        if "error" in data:
            raise RuntimeError(f"{method}: {data['error']}")
        return data.get("result")

    def lua(self, code: str):
        res = self.call("viber.lua", {"code": code})
        if not res or not res.get("ok"):
            raise RuntimeError((res or {}).get("error") or f"lua falhou: {res}")
        return res.get("result")

    def ready(self) -> bool:
        try:
            ping = self.call("viber.ping", timeout=5)
        except Exception:
            return False
        ready = ping.get("ready") if isinstance(ping, dict) else None
        if isinstance(ready, dict):
            return bool(ready.get("ready"))
        return bool(ready)

    def wait_ready(self, timeout: float) -> bool:
        end = time.time() + timeout
        while time.time() < end:
            if self.ready():
                return True
            time.sleep(1.0)
        return False


def screenshot(port: int, out: Path) -> bool:
    """`viber debug screenshot` (faz o polling da captura)."""
    res = subprocess.run(
        ["viber", "debug", "--port", str(port), "screenshot", "-o", str(out)],
        capture_output=True,
        text=True,
        timeout=90,
    )
    return res.returncode == 0 and out.exists()


NAMED_KEYS = {
    "space": "Space", "spacebar": "Space", "enter": "Enter", "return": "Enter",
    "esc": "Escape", "escape": "Escape", "tab": "Tab", "backspace": "Backspace",
    "delete": "Delete", "up": "ArrowUp", "down": "ArrowDown", "left": "ArrowLeft",
    "right": "ArrowRight", "shift": "ShiftLeft", "ctrl": "ControlLeft", "alt": "AltLeft",
}


def key_code(raw: str) -> str:
    """`e` → `KeyE`, `1` → `Digit1`, `f2` → `F2` (o `normalize_key` do Rust)."""
    low = raw.strip().lower()
    if low in NAMED_KEYS:
        return NAMED_KEYS[low]
    if re.fullmatch(r"f\d{1,2}", low):
        return low.upper()
    if len(low) == 1 and low.isalpha():
        return f"Key{low.upper()}"
    if len(low) == 1 and low.isdigit():
        return f"Digit{low}"
    return raw.strip()


def short(err: object, limit: int = 240) -> str:
    text = str(err)
    return text if len(text) <= limit else text[:limit] + "…"


def lua_str(value: str) -> str:
    # String JSON = literal Luau válido, desde que sem `\uXXXX` (o Luau só
    # aceita `\u{…}`): os acentos vão em UTF-8 cru.
    return json.dumps(value, ensure_ascii=False)


RULES = 'local r = viber.load("lib/rules.lua")\n'


def live_step(bridge: Bridge, step: dict, shots: Path) -> str:
    """Executa um passo no jogo vivo. Devolve o detalhe; levanta em falha."""
    (kind, arg), = step.items()
    if kind == "accept":
        ok = bridge.lua(RULES + f"return r.quest_accept({lua_str(arg)})")
        if not ok:
            raise RuntimeError(f"{arg}: não aceitou")
        return "active"
    if kind == "kill":
        a = {"kind": arg} if isinstance(arg, str) else arg
        n, elite = int(a.get("count", 1)), "true" if a.get("elite") else "false"
        bridge.lua(RULES + f"for _ = 1, {n} do r.report_kill({lua_str(a['kind'])}, {elite}) end return true")
        return f"{a['kind']} ×{n}"
    if kind == "slay":
        a = {"kind": arg} if isinstance(arg, str) else arg
        want, n = a["kind"], int(a.get("count", 1))
        killed = 0
        deadline = time.time() + float(a.get("timeout", 20))
        # As criaturas entram no registo `enemies` quando o script delas
        # corre (perto do herói); o golpe é o dano do jogo, a morte e o
        # crédito na quest vêm do script da criatura no frame seguinte.
        while killed < n and time.time() < deadline:
            killed += int(bridge.lua(
                RULES + f"local want, left = {lua_str(want)}, {n - killed}\n"
                "local hit = 0\n"
                "for _, h in ipairs(r.hostiles_all()) do\n"
                "  if hit >= left then break end\n"
                "  if r.kind_matches(h.kind, want) then\n"
                "    if r.damage_enemy(h.id, 1e6) then hit = hit + 1 end\n"
                "  end\n"
                "end\n"
                "return hit"
            ) or 0)
            if killed < n:
                time.sleep(0.5)
        time.sleep(1.0)
        if killed < n:
            raise RuntimeError(f"só {killed}/{n} {want} vivos na cena")
        return f"{want} ×{killed}"
    if kind in ("collect", "craft"):
        a = {"item": arg} if isinstance(arg, str) else arg
        item, n = lua_str(a["item"]), int(a.get("count", 1))
        if kind == "collect":
            bridge.lua(RULES + f"r.report_collect({item}, {n}) return true")
        else:
            # A forja real (`lib/craft.lua`): gasta a receita ou falha.
            made = bridge.lua(
                'local craft = viber.load("lib/craft.lua")\n'
                f"local want = {item}\n"
                "local id\n"
                "for _, rec in ipairs(craft.recipes()) do\n"
                "  if rec.out == want or rec.id == want then id = rec.id break end\n"
                "end\n"
                "if not id then return -1 end\n"
                f"local made = 0 for _ = 1, {n} do if craft.make(id) then made = made + 1 end end\n"
                "return made"
            )
            if made == -1:
                raise RuntimeError(f"{a['item']}: sem receita")
            if made < n:
                raise RuntimeError(f"forja de {a['item']}: {made}/{n} (faltam materiais?)")
        return f"{a['item']} ×{n}"
    if kind == "visit":
        bridge.lua(RULES + f"r.report_visit({lua_str(arg)}) return true")
        return arg
    if kind == "progress":
        bridge.lua(RULES + f"return r.quest_set_progress({lua_str(arg['quest'])}, {int(arg['n'])})")
        return f"{arg['quest']} = {arg['n']}"
    if kind == "turn_in":
        if not bridge.lua(RULES + f"return r.quest_turn_in({lua_str(arg)})"):
            raise RuntimeError(f"{arg}: entrega recusada")
        return "entregue"
    if kind == "expect":
        res = bridge.lua(f"return viber.vael.qa_check({lua_str(json.dumps(arg))})")
        if not res.get("ok"):
            raise RuntimeError("; ".join(res.get("failures") or []))
        return "ok"
    if kind == "lua":
        return json.dumps(bridge.lua(arg))[:200]
    if kind == "key":
        bridge.call("viber.input.key", {"key": key_code(arg)})
        return arg
    if kind == "wait":
        time.sleep(float(arg))
        return f"{arg}s"
    if kind == "teleport":
        x, y, z = arg
        bridge.lua(f"viber.teleport_player({x}, {y}, {z}) return true")
        return f"{x} {y} {z}"
    if kind in ("goto", "interact"):
        pos = bridge.lua(
            f"local id = viber.debug.find({lua_str(arg)}) if not id then return nil end "
            "return { viber.debug.pos(id) }"
        )
        if not pos:
            raise RuntimeError(f"entidade {arg!r} não existe")
        x, y, z = pos
        # 1,2 m: o [E] só chega a metade do alcance autorado (3,5 → 1,75 m).
        bridge.lua(f"viber.teleport_player({x}, {y + 0.5}, {z - 1.2}) return true")
        if kind == "interact":
            # O [E] logo a seguir a um teleporte/arranque pode cair num frame
            # sem foco de interação (o herói ainda a assentar) e ir para a
            # cura. Confirma pelo estado do jogo e repete até 3 vezes.
            time.sleep(1.3)
            before = bridge.lua("return viber.vael.qa_snapshot()")
            for attempt in range(1, 4):
                bridge.call("viber.input.key", {"key": "KeyE"})
                time.sleep(0.7)
                if bridge.lua("return viber.vael.qa_snapshot()") != before:
                    return f"{arg} @ {x:.1f} {z:.1f} ([E] ×{attempt})"
            return f"{arg} @ {x:.1f} {z:.1f} (sem mudança de estado)"
        return f"{arg} @ {x:.1f} {z:.1f}"
    if kind == "say":
        bridge.call("viber.input.text", {"text": arg})
        bridge.call("viber.input.key", {"key": "Enter"})
        return arg
    if kind == "screenshot":
        out = shots / f"{arg}.png"
        if not screenshot(bridge.port, out):
            raise RuntimeError("screenshot falhou")
        return str(out)
    raise RuntimeError(f"passo desconhecido: {kind}")


# ── cenários ────────────────────────────────────────────────────────────


def cmd_list(_args) -> None:
    rows = []
    for path in sorted(SCEN_DIR.rglob("*.json")):
        try:
            data = json.loads(path.read_text())
        except json.JSONDecodeError as err:
            rows.append((path.relative_to(SCEN_DIR).with_suffix(""), "?", f"JSON inválido: {err}"))
            continue
        rows.append(
            (
                path.relative_to(SCEN_DIR).with_suffix(""),
                data.get("scene", "(herda)" if data.get("extends") else "-"),
                data.get("description", ""),
            )
        )
    active = ACTIVE.read_text().strip() if ACTIVE.exists() else ""
    for name, scene, desc in rows:
        mark = "*" if str(name) == active else " "
        print(f"{mark} {str(name):34} {scene:18} {desc}")


def steps_for(quest: dict, from_stage: int, state: str, live_kills: bool = False) -> tuple[list[dict], dict]:
    """Passos lógicos que levam a quest do estado pedido à entrega."""
    steps: list[dict] = []
    qid = quest["id"]
    recipes = {r["out"]: r for r in json.loads((ROOT / "data" / "recipes.json").read_text())["recipes"]}
    bag: dict[str, int] = dict(json.loads((ROOT / "data" / "items.json").read_text()).get("starting", {}))
    if state == "not_taken":
        steps.append({"accept": qid})
    if state in ("not_taken", "active"):
        for stage in stages(quest)[from_stage - 1 :]:
            kind, target, count = stage.get("type"), stage.get("target", ""), int(stage.get("count", 1) or 1)
            if kind == "kill":
                steps.append({"slay" if live_kills else "kill": {"kind": target, "count": count}})
            elif kind == "collect":
                steps.append({"collect": {"item": target, "count": count}})
                bag[target] = bag.get(target, 0) + count
            elif kind == "craft":
                # Os materiais da receita que a mochila ainda não tem.
                for item, n in (recipes.get(target) or {}).get("cost", {}).items():
                    missing = n * count - bag.get(item, 0)
                    if missing > 0:
                        steps.append({"collect": {"item": item, "count": missing}})
                        bag[item] = bag.get(item, 0) + missing
                steps.append({"craft": {"item": target, "count": count}})
            elif kind == "visit":
                for place in target.split()[:count] or [target]:
                    steps.append({"visit": place})
            elif kind == "decision":
                steps.append({"progress": {"quest": qid, "n": count}})
        steps.append({"expect": {"quests": {qid: "ready"}}})
    steps.append({"turn_in": qid})
    expect = {"quests": {qid: "done"}}
    gold = (quest.get("rewards") or {}).get("gold")
    if gold:
        expect["items"] = {"gold": f">={gold}"}
    return steps, expect


def cmd_new(args) -> None:
    quests = load_quests()
    if args.quest not in quests:
        die(f"quest desconhecida: {args.quest}")
    quest = quests[args.quest]
    path = scenario_path(args.name)
    if path.exists() and not args.force:
        die(f"{path.relative_to(ROOT)} já existe (--force)")
    live_kills = bool(getattr(args, "slay", False))
    steps, expect = steps_for(quest, args.stage or 1, args.state, live_kills)
    scenario: dict = {
        "description": args.description or f"{quest.get('title', args.quest)} — {args.state}"
        + (f", etapa {args.stage}" if args.stage else ""),
        "extends": ["_base"],
        "scene": args.scene,
        "quest": args.quest,
        "state": args.state,
    }
    if args.stage:
        scenario["stage"] = args.stage
    scenario["player"] = {"near": args.near or "@giver"}
    if live_kills:
        scenario["player"]["god"] = True  # os inimigos da cena atacam
    if args.live:
        # Aceitar e entregar pelo diálogo [E] do dador, como o jogador.
        talk = {"interact": "@giver"}
        steps = [talk if ("accept" in s or "turn_in" in s) else s for s in steps]
    scenario["steps"] = steps
    scenario["expect"] = expect
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(scenario, ensure_ascii=False, indent=2) + "\n")
    print(path.relative_to(ROOT))


def cmd_sim(args) -> None:
    cmd = vael_cmd("--qa-sim", args.name, *(["--json"] if args.json else []))
    sys.exit(subprocess.call(cmd, cwd=ROOT))


def cmd_use(args) -> None:
    if not scenario_path(args.name).exists():
        die(f"cenário inexistente: {args.name}")
    ACTIVE.parent.mkdir(parents=True, exist_ok=True)
    ACTIVE.write_text(args.name + "\n")
    print(f"activo: {args.name} — no Jackdaw: Play ▸ \"Play (QA)\"")


def resolve_giver(steps: list[dict], scenario_quest: str | None) -> list[dict]:
    quests = load_quests()
    giver = (quests.get(scenario_quest or "") or {}).get("npc") or f"npc_{scenario_quest}"
    out = []
    for step in steps:
        (kind, arg), = step.items()
        if kind in ("goto", "interact") and arg == "@giver":
            step = {kind: giver}
        out.append(step)
    return out


def merge_expect(into: dict, extra: dict) -> None:
    for key, value in extra.items():
        if isinstance(value, dict):
            into.setdefault(key, {}).update(value)
        else:
            into[key] = value


def merged(name: str, seen: set | None = None) -> dict:
    """`extends` resolvido em Python (só para os passos vivos e o expect)."""
    seen = set() if seen is None else seen
    if name in seen:
        die(f"extends em ciclo: {name}")
    seen.add(name)
    data = json.loads(scenario_path(name).read_text())
    out: dict = {"steps": [], "expect": {}}
    for parent in data.get("extends", []):
        base = merged(parent, seen)
        out["steps"] += base.pop("steps")
        merge_expect(out["expect"], base.pop("expect"))
        out.update(base)
    out.update({k: v for k, v in data.items() if k not in ("steps", "expect", "extends")})
    out["steps"] += data.get("steps", [])
    merge_expect(out["expect"], data.get("expect", {}))
    return out


def run_steps(bridge: Bridge, scen: dict, logs: Path, keep_going: bool) -> list[str]:
    """Passos vivos + `expect` final + screenshot. Devolve as falhas."""
    failures: list[str] = []
    for i, step in enumerate(resolve_giver(scen.get("steps", []), scen.get("quest")), 1):
        label = json.dumps(step, ensure_ascii=False)
        try:
            detail = live_step(bridge, step, logs)
            print(f"  ok   {i:2} {label} {detail}")
        except Exception as err:  # noqa: BLE001 — relatório, não crash
            print(f"  FAIL {i:2} {label} {short(err)}")
            failures.append(f"passo {i}: {short(err)}")
            if not keep_going:
                break
    res = bridge.lua("return viber.vael.qa_check()")
    failures += res.get("failures") or []
    screenshot(bridge.port, logs / "final.png")
    return failures


def cmd_steps(args) -> None:
    """Os passos de um cenário num jogo já a correr (reaplica antes)."""
    bridge = Bridge(args.port)
    if not args.no_apply:
        bridge.lua(f"return viber.vael.qa_apply({lua_str(args.name)})")
        time.sleep(1.0)
    logs = Path(tempfile.mkdtemp(prefix=f"vael-qa-{Path(args.name).name}-"))
    failures = run_steps(bridge, merged(args.name), logs, args.keep_going)
    print(f"{'PASS' if not failures else 'FAIL'} {args.name} (artefactos em {logs})")
    for failure in failures:
        print(f"  ✗ {failure}")
    sys.exit(0 if not failures else 1)


def cmd_run(args) -> None:
    scen = merged(args.name)
    port = args.port
    bridge = Bridge(port)
    if bridge.ready():
        die(f"já há um jogo na porta {port} (usa --port ou qa.py apply)")
    logs = Path(tempfile.mkdtemp(prefix=f"vael-qa-{Path(args.name).name}-"))
    log = open(logs / "game.log", "w")
    if args.no_build:
        exe = ROOT / "target" / ("debug" if args.debug else "release") / "vael"
        if not exe.exists():
            die(f"{exe} não existe (sem --no-build compila)")
    else:
        exe = build_game(release=not args.debug, log=log)
    cmd = [str(exe), "world.bsn", "--scenario", args.name, "--bridge", str(port)]
    if args.resolution:
        cmd += ["--resolution", args.resolution]
    print(f"qa: a arrancar ({' '.join(cmd[1:])}); log em {logs / 'game.log'}")
    t0 = time.time()
    game = subprocess.Popen(cmd, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    failures: list[str] = []
    try:
        if not bridge.wait_ready(args.timeout):
            if game.poll() is not None:
                die(f"o jogo saiu (código {game.returncode}); ver {logs / 'game.log'}", 1)
            die(f"sem `ready` em {args.timeout:.0f}s; ver {logs / 'game.log'}", 1)
        print(f"qa: pronto em {time.time() - t0:.1f}s")
        # O cenário aplica-se 3 frames depois do `ready`.
        time.sleep(1.0)
        failures += run_steps(bridge, scen, logs, args.keep_going)
    finally:
        if args.keep:
            print(f"qa: jogo a correr (pid {game.pid}, porta {port}); `qa.py apply/check --port {port}`")
        elif game.poll() is None:
            try:
                os.killpg(game.pid, signal.SIGTERM)
                game.wait(timeout=15)
            except subprocess.TimeoutExpired:
                os.killpg(game.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
    status = "PASS" if not failures else "FAIL"
    print(f"{status} {args.name} ({time.time() - t0:.0f}s; artefactos em {logs})")
    for failure in failures:
        print(f"  ✗ {failure}")
    sys.exit(0 if not failures else 1)


def cmd_apply(args) -> None:
    name = lua_str(args.name) if args.name else "nil"
    Bridge(args.port).lua(f"return viber.vael.qa_apply({name})")
    print("reaplicado (no frame seguinte)")


def cmd_check(args) -> None:
    res = Bridge(args.port).lua("return viber.vael.qa_check()")
    for failure in res.get("failures") or []:
        print(f"  ✗ {failure}")
    print("PASS" if res.get("ok") else "FAIL")
    sys.exit(0 if res.get("ok") else 1)


def cmd_snapshot(args) -> None:
    bridge = Bridge(args.port)
    text = bridge.lua("return viber.vael.qa_snapshot()")
    data = json.loads(text)
    pos = bridge.lua("local ok, x, y, z = viber.player_position() if ok then return {x, y, z} end return nil")
    scene = bridge.lua("return viber.scene_name and viber.scene_name() or nil")
    out = {"description": args.description or f"instantâneo de {scene}", "scene": scene, **data}
    if pos:
        out.setdefault("player", {})["pos"] = [round(v, 2) for v in pos]
    path = scenario_path(args.name)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(out, ensure_ascii=False, indent=2) + "\n")
    print(path.relative_to(ROOT))


def cmd_state(args) -> None:
    text = Bridge(args.port).lua("return viber.vael.qa_snapshot()")
    data = json.loads(text)
    for key in ("quests", "items", "gear", "skills", "level", "hp", "biomes"):
        if data.get(key):
            print(f"{key}: {json.dumps(data[key], ensure_ascii=False)}")


def cmd_lua(args) -> None:
    print(json.dumps(Bridge(args.port).lua(args.code), ensure_ascii=False, indent=2))


# ── cenas ───────────────────────────────────────────────────────────────

NAME_RE = re.compile(r'^(\s*)#"?([^"\n]+?)"?\s*$')


def find_block(name: str) -> tuple[Path, list[str]] | None:
    """Subárvore `.bsn` da entidade `name` (o 1.º ficheiro que a tem)."""
    for path in WORLD_FILES:
        lines = path.read_text().splitlines()
        for i, line in enumerate(lines):
            m = NAME_RE.match(line)
            if not m or m.group(2) != name:
                continue
            indent = len(m.group(1))
            block = [line]
            # Os `Children [` … `]` da própria entidade ficam na MESMA
            # indentação do `#nome`; só um `--`/`]` fora deles fecha o bloco.
            depth = 0
            for nxt in lines[i + 1 :]:
                stripped = nxt.strip()
                lead = len(nxt) - len(nxt.lstrip())
                if stripped and lead < indent:
                    break
                if stripped and lead == indent:
                    if stripped.endswith("["):
                        depth += 1
                    elif stripped == "]":
                        if depth == 0:
                            break
                        depth -= 1
                    elif stripped == "--" and depth == 0:
                        break
                block.append(nxt)
            return path, [ln[indent:] if ln.strip() else "" for ln in block]
    return None


def place_block(block: list[str], x: float, z: float, indent: int) -> list[str]:
    """Muda a pose da raiz (Transform e attr `translation`) e re-indenta."""
    out = []
    head = True
    moved_tf = moved_attr = False
    for line in block:
        if head and line.startswith("bevy_ecs::hierarchy::Children"):
            head = False
        if head and not moved_tf and "translation: glam::Vec3" in line:
            line = re.sub(r"glam::Vec3 \{[^}]*\}", f"glam::Vec3 {{ x: {x:.6f}, y: 0.0, z: {z:.6f} }}", line)
            moved_tf = True
        if head and not moved_attr and 'key: "translation"' in line:
            line = re.sub(r'value: "[^"]*"', f'value: "{x:g} 0 {z:g}"', line)
            moved_attr = True
        # Ids frescos: a cópia é outro nó (as edições ao vivo do Jackdaw
        # seguem o `SceneNodeId`).
        line = re.sub(r"SceneNodeId\(\d+\)", lambda _: f"SceneNodeId({random.getrandbits(62)})", line)
        out.append((" " * indent + line) if line else "")
    return out


def find_entities(text: str) -> list[tuple[str, str]]:
    hits = []
    for path in WORLD_FILES:
        for line in path.read_text().splitlines():
            m = NAME_RE.match(line)
            if m and text.lower() in m.group(2).lower() and not m.group(2).startswith(
                ("GltfScene", "GLTFLoader", "célula")
            ):
                hits.append((m.group(2), str(path.relative_to(ROOT))))
    return hits


def creature_urls() -> dict[str, str]:
    """`enemies/<kind>` e `bosses/<kind>` → modelo que o mundo usa."""
    urls: dict[str, str] = {}
    for path in (ROOT / "world" / "creatures").glob("*.bsn"):
        text = path.read_text()
        for m in re.finditer(r'path: "((?:enemies|bosses)/[\w-]+)\.lua"', text):
            u = re.search(r'key: "url", value: "([^"]+)"', text[m.end() : m.end() + 4000])
            if u:
                urls.setdefault(m.group(1), u.group(1))
    return urls


def scene_xml(title: str, size: float, enemies: list[tuple[str, int]], interior: bool) -> str:
    urls = creature_urls()
    half = size / 2
    parts = [
        '<?xml version="1.0" encoding="UTF-8"?>',
        f'<world static="true" clear-color="#9db7cf" title="{title}">',
    ]
    if interior:
        parts.append(f'  <InteriorScene at="0 0" size="{size:g} {size:g}" camera-distance="16" camera-pitch="55" camera-yaw="0" />')
    parts += [
        '  <SceneEntry name="qa" translation="0 0.6 -6" euler="0 0 0" />',
        "  <Entity>",
        '    <AmbientLight color="0xfff2e0" brightness="500" />',
        '    <DirectionalLight euler="-55 35 0" illuminance="18000" shadows="true" />',
        "  </Entity>",
        '  <Composition name="qa.ground" translation="0 0 0" body="fixed" collider="auto">',
        f'    <Box texture="/assets/textures/dirt_road/albedo.ktx2" texture-tile-size="4.0" translation="0 -0.1 0" half-size="{half:g} 0.1 {half:g}" />',
        "  </Composition>",
    ]
    # Criaturas como entidades fixas (o formato dos chefes): o
    # `DynamicSpawner` espera pelo terreno e numa cena sem terreno segurava o
    # véu até ao teto de 240 s.
    for i, (kind, count) in enumerate(enemies):
        script = kind if "/" in kind else f"enemies/{kind}"
        url = urls.get(script, f"/assets/meshes/characters/{kind.split('/')[-1].split('-')[0]}_lod0.glb")
        z = 10 + i * 5
        for j in range(count):
            x = (j - (count - 1) / 2) * 3.0
            name = f"qa.{kind.replace('/', '.')}.{j + 1}"
            parts += [
                f'  <Entity name="{name}" translation="{x:g} 0 {z}" euler="0 180 0" script="{script}.lua" hostile="true" health="100" ragdoll="true">',
                f'    <GltfScene url="{url}" />',
                "  </Entity>",
            ]
    parts.append("</world>")
    return "\n".join(parts) + "\n"


def migrate(path: Path) -> None:
    """`bsn_migrate --props --node-ids`: specs tipados e ids de nó."""
    for profile in ("release", "debug"):
        exe = EXT / "target" / profile / "examples" / "bsn_migrate"
        if exe.exists():
            subprocess.run([str(exe), "--props", "--node-ids", str(path)], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
            return
    subprocess.run(
        ["cargo", "run", "-q", "-p", "bevy_ext_world", "--example", "bsn_migrate", "--", "--props", "--node-ids", str(path)],
        cwd=EXT,
        check=True,
        stdout=subprocess.DEVNULL,
    )


def build_scene(name: str, copies: list[str], enemies: list[str], size: float, interior: bool, force: bool) -> Path:
    out = SCENE_DIR / f"{name}.bsn"
    if out.exists() and not force:
        die(f"{out.relative_to(ROOT)} já existe (--force)")
    if not shutil.which("viber"):
        die("`viber` não está no PATH (../viber-tools/tools/install.sh)")
    parsed_enemies = []
    for spec in enemies:
        kind, _, n = spec.partition(":")
        parsed_enemies.append((kind, int(n or 3)))
    SCENE_DIR.mkdir(parents=True, exist_ok=True)
    xml = SCENE_DIR / f"{name}.xml"
    xml.write_text(scene_xml(f"QA {name}", size, parsed_enemies, interior))
    try:
        subprocess.run(["viber", "bsn", "convert", "--force", str(xml)], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
    finally:
        xml.unlink(missing_ok=True)
    lines = out.read_text().splitlines()
    # Último `]` = fecho dos filhos da raiz.
    close = max(i for i, ln in enumerate(lines) if ln.strip() == "]")
    inserted: list[str] = []
    for i, spec in enumerate(copies):
        ent, _, at = spec.partition("@")
        if at:
            x, z = (float(v) for v in at.split(","))
        else:
            x, z = (i - (len(copies) - 1) / 2) * 4.0, 4.0
        found = find_block(ent)
        if not found:
            die(f"entidade {ent!r} não existe no mundo (qa.py scene find {ent})")
        src, block = found
        inserted += ["    --", *place_block(block, x, z, 4)]
        print(f"  + {ent} de {src.relative_to(ROOT)} em ({x:g}, {z:g})")
    lines[close:close] = inserted
    out.write_text("\n".join(lines) + "\n")
    migrate(out)
    return out


def cmd_scene_new(args) -> None:
    out = build_scene(args.name, args.copy or [], args.enemy or [], args.size, args.interior, args.force)
    print(f"{out.relative_to(ROOT)} — cena `qa/{args.name}`")


def cmd_scene_find(args) -> None:
    for name, path in find_entities(args.text)[:80]:
        print(f"{name:40} {path}")


# Entidades que cada tipo de etapa pede na cena.
STAGE_PROPS = {"craft": ["anvil"]}


def cmd_scene_quest(args) -> None:
    quests = load_quests()
    quest = quests.get(args.quest) or die(f"quest desconhecida: {args.quest}")
    copies = [quest.get("npc") or f"npc_{args.quest}"]
    enemies = []
    for stage in stages(quest):
        copies += [p for p in STAGE_PROPS.get(stage.get("type"), []) if p not in copies]
        if stage.get("type") == "kill" and stage.get("target") not in ("elite", "boss"):
            enemies.append(f"{stage['target']}:{max(2, int(stage.get('count', 1)))}")
    name = args.name or args.quest.replace("_", "-")
    if copies[0] == "notice_board":
        copies = copies[1:]
    build_scene(name, copies, enemies, args.size, False, args.force)
    for state in ("not_taken", "active"):
        sname = f"{args.quest}/{state}"
        ns = argparse.Namespace(
            name=sname, quest=args.quest, state=state, stage=None, scene=f"qa/{name}",
            near="@giver", description=None, live=False, force=args.force, slay=bool(enemies),
        )
        if not scenario_path(sname).exists() or args.force:
            cmd_new(ns)
    print(f"cena qa/{name} + cenários {args.quest}/not_taken e {args.quest}/active")


# ── CLI ─────────────────────────────────────────────────────────────────


def main() -> None:
    ap = argparse.ArgumentParser(prog="qa.py", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    def port(p):
        p.add_argument("--port", type=int, default=DEFAULT_PORT)

    sub.add_parser("list").set_defaults(func=cmd_list)

    p = sub.add_parser("new")
    p.add_argument("name")
    p.add_argument("--quest", required=True)
    p.add_argument("--state", default="active", choices=["not_taken", "active", "ready", "done"])
    p.add_argument("--stage", type=int)
    p.add_argument("--scene", default="main")
    p.add_argument("--near")
    p.add_argument("--description")
    p.add_argument("--live", action="store_true", help="começa por falar com o dador ([E])")
    p.add_argument("--slay", action="store_true", help="abates com criaturas reais (cena com inimigos)")
    p.add_argument("--force", action="store_true")
    p.set_defaults(func=cmd_new)

    p = sub.add_parser("sim")
    p.add_argument("name", nargs="?", default="all")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_sim)

    p = sub.add_parser("use")
    p.add_argument("name")
    p.set_defaults(func=cmd_use)

    p = sub.add_parser("run")
    p.add_argument("name")
    port(p)
    p.add_argument("--timeout", type=float, default=300)
    p.add_argument("--keep", action="store_true", help="deixa o jogo a correr")
    p.add_argument("--keep-going", action="store_true", help="não pára no 1.º passo falhado")
    p.add_argument("--debug", action="store_true", help="binário debug")
    p.add_argument("--no-build", action="store_true", help="usa o binário já compilado (build partido por outros)")
    p.add_argument("--resolution", default="1280x720")
    p.set_defaults(func=cmd_run)

    p = sub.add_parser("steps")
    p.add_argument("name")
    port(p)
    p.add_argument("--no-apply", action="store_true", help="não reaplica o estado antes")
    p.add_argument("--keep-going", action="store_true")
    p.set_defaults(func=cmd_steps)

    p = sub.add_parser("apply")
    p.add_argument("name", nargs="?")
    port(p)
    p.set_defaults(func=cmd_apply)

    p = sub.add_parser("check")
    port(p)
    p.set_defaults(func=cmd_check)

    p = sub.add_parser("snapshot")
    p.add_argument("name")
    p.add_argument("--description")
    port(p)
    p.set_defaults(func=cmd_snapshot)

    p = sub.add_parser("state")
    port(p)
    p.set_defaults(func=cmd_state)

    p = sub.add_parser("lua")
    p.add_argument("code")
    port(p)
    p.set_defaults(func=cmd_lua)

    sc = sub.add_parser("scene").add_subparsers(dest="scmd", required=True)
    p = sc.add_parser("new")
    p.add_argument("name")
    p.add_argument("--copy", action="append", help="entidade do mundo (nome[@x,z])")
    p.add_argument("--enemy", action="append", help="kind:count (scripts/enemies/<kind>.lua)")
    p.add_argument("--size", type=float, default=60)
    p.add_argument("--interior", action="store_true", help="câmara de sala (InteriorScene)")
    p.add_argument("--force", action="store_true")
    p.set_defaults(func=cmd_scene_new)
    p = sc.add_parser("find")
    p.add_argument("text")
    p.set_defaults(func=cmd_scene_find)
    p = sc.add_parser("quest")
    p.add_argument("quest")
    p.add_argument("--name")
    p.add_argument("--size", type=float, default=60)
    p.add_argument("--force", action="store_true")
    p.set_defaults(func=cmd_scene_quest)

    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
