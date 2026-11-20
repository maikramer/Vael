#!/usr/bin/env python3
"""Oráculo visual+estado do Vael com o modelo de decisão `liquid/d1-ultra`.

Camada 2 do playtest (a camada 1 é o `bevy_playtest`, em
`tests/playtest_scenarios.rs`). O jogo real corre a 640x360 com um cenário QA;
este processo:

1. captura PNG a ~10 Hz pelo bridge (`viber.screenshot`/`viber.screenshot_status`);
2. a cada 1-2 s envia os últimos 8 frames + o estado (`qa_snapshot`, posição,
   cena, FPS) para a Decisions API da OpenRouter;
3. pede quatro respostas tipadas (`visual_ok`, `estado_ok`, `anomalia`,
   `severidade`) e guarda um artefacto quando o veredicto é "não conforme".

Uso:
    python3 tools/playtest_vision.py --scenario combat/arena --duration 90
    python3 tools/playtest_vision.py --tour qa/vision/tours/forge_interior.json
    python3 tools/playtest_vision.py --attach 15740 --duration 120
    python3 tools/playtest_vision.py --probe
    python3 tools/playtest_vision.py --self-test

A chave lê-se de `OPENROUTER_API_KEY` (ambiente) ou `secrets.env` na raiz.
Sem dependências fora da stdlib; o cliente do jogo é o `qa.py`.
"""
from __future__ import annotations

import argparse
import base64
import itertools
import json
import os
import signal
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from collections import deque
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
import qa  # noqa: E402  (o módulo vive ao lado; sem side effects no import)

DEFAULT_ENDPOINT = os.environ.get(
    "OPENROUTER_DECISIONS_URL", "https://openrouter.ai/api/alpha/decisions"
)
DEFAULT_MODEL = os.environ.get("OPENROUTER_DECISION_MODEL", "liquid/d1-ultra")
DEFAULT_RESOLUTION = "480x270"
DEFAULT_CAPTURE_HZ = 10.0
DEFAULT_DECISION_HZ = 1.5
DEFAULT_FRAMES = 8
MAX_FRAMES = 8  # limite do Liquid d1/d1-ultra
DEFAULT_BUDGET_USD = 0.50
DEFAULT_MIN_CONFIDENCE = 0.6
DEFAULT_MAX_BODY_MB = 4.0
DEFAULT_DURATION = 60.0
CAPTURE_POLL_S = 0.01
CAPTURE_TIMEOUT_S = 5.0
# Caminho rápido: `brp_extras/screenshot` devolve depois de publicar o PNG
# (~40 ms a 480x270). O `viber.screenshot`+status legado leva ~250-300 ms
# (fila da engine + leitura/base64 do ficheiro) e fica como fallback.
_CAPTURE_DIR = Path(tempfile.mkdtemp(prefix="vael-playtest-"))
_CAPTURE_SEQ = itertools.count()
_BRP_SCREENSHOT = True

QUESTIONS = {
    "visual_ok": {
        "type": "noul",
        "instructions": (
            "The attached frames were captured from a third-person fantasy game. "
            "Does the game render correctly in these frames? Answer true only when "
            "there is no visible rendering or presentation defect."
        ),
        "criteria": {
            "true": (
                "The world and UI render correctly: terrain and props are textured, "
                "the character is posed normally, the HUD is visible and legible, and "
                "the camera is outside geometry."
            ),
            "false": (
                "At least one visible defect: black/white/blank screen, magenta or "
                "checkerboard missing textures, exploded or stretched geometry, a "
                "character in T-pose or visibly detached limbs, objects floating or "
                "sunk into the ground, broken/overlapping/missing UI, or the camera "
                "inside walls/terrain."
            ),
        },
    },
    "estado_ok": {
        "type": "noul",
        "instructions": (
            "The text state comes from the same game run (health, position, quests, "
            "inventory and frame timing). Is the state internally coherent for this "
            "scene and moment?"
        ),
        "criteria": {
            "true": (
                "Health is between zero and its maximum, the player position is a "
                "finite plausible point in the scene, quest and inventory values are "
                "sane, and there is no evidence of a frozen or corrupted game."
            ),
            "false": (
                "The state is impossible or contradictory: health above its maximum or "
                "negative, a non-finite/obviously out-of-world position, a quest in an "
                "unknown state, negative item counts, or frame timing indicating a "
                "freeze."
            ),
        },
    },
    "anomalia": {
        "type": "choice",
        "instructions": (
            "If the frames or state show a defect, pick the dominant category. Pick "
            "'none' when everything conforms."
        ),
        "criteria": {
            "none": "No defect is visible and the state is coherent.",
            "black_screen": "The view is black, white, blank or otherwise not rendering.",
            "missing_texture": "Magenta, checkerboard or untextured surfaces.",
            "geometry_explosion": "Exploded, stretched, flickering or badly clipped geometry.",
            "t_pose": "A character is in T-pose, has detached limbs or is otherwise unanimated.",
            "floating_object": "Props or characters float above, sink into, or intersect the ground.",
            "ui_broken": "HUD/menu is missing, overlapping, clipped, unreadable or mispositioned.",
            "camera_clipping": "The camera is inside terrain, walls or a character.",
            "other": "A defect that does not fit the other categories.",
        },
    },
    "severidade": {
        "type": "score",
        "instructions": "How severe is the defect, when there is one?",
        "criteria": [
            "Cosmetic: visible but does not affect play.",
            "Gameplay impact: obscures play, blocks an interaction or breaks feedback.",
            "Blocking: crash-like, soft-lock, unplayable view or lost state.",
        ],
    },
}


# ── segredos ────────────────────────────────────────────────────────────


def load_secrets(path: str | None = None) -> str:
    """Lê a chave do ambiente ou de `secrets.env`; nunca a imprime."""
    if os.environ.get("OPENROUTER_API_KEY"):
        return os.environ["OPENROUTER_API_KEY"]
    candidates = []
    if path:
        candidates.append(Path(path))
    elif os.environ.get("VAEL_SECRETS_FILE"):
        candidates.append(Path(os.environ["VAEL_SECRETS_FILE"]))
    candidates.append(ROOT / "secrets.env")
    for candidate in candidates:
        if not candidate.is_file():
            continue
        mode = candidate.stat().st_mode & 0o777
        if mode & 0o077:
            log(f"aviso: {candidate} tem permissões {oct(mode)}; usa chmod 600")
        for raw in candidate.read_text(encoding="utf-8").splitlines():
            line = raw.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            key, value = line.split("=", 1)
            key, value = key.strip(), value.strip().strip("'\"")
            if key and value:
                os.environ.setdefault(key, value)
    key = os.environ.get("OPENROUTER_API_KEY", "").strip()
    if not key:
        die(
            "falta OPENROUTER_API_KEY (define-a no ambiente ou em secrets.env; "
            "vê secrets.env.example)"
        )
    return key


def log(message: str) -> None:
    print(f"playtest-vision: {message}", file=sys.stderr, flush=True)


def die(message: str, code: int = 2) -> None:
    log(message)
    raise SystemExit(code)


# ── decisões ────────────────────────────────────────────────────────────


class DecisionError(RuntimeError):
    def __init__(self, message: str, *, fatal: bool = False):
        super().__init__(message)
        self.fatal = fatal


class DecisionClient:
    def __init__(
        self,
        api_key: str,
        model: str = DEFAULT_MODEL,
        endpoint: str = DEFAULT_ENDPOINT,
        timeout: float = 60.0,
        retries: int = 3,
    ):
        self.api_key = api_key
        self.model = model
        self.endpoint = endpoint
        self.timeout = timeout
        self.retries = retries
        self.cost = 0.0
        self.requests = 0
        self.last_usage: dict = {}

    def ask(self, state, questions: dict, session_id: str | None = None) -> dict:
        body = {"model": self.model, "state": state, "questions": questions}
        if session_id:
            body["session_id"] = session_id
        data = json.dumps(body, ensure_ascii=False).encode("utf-8")
        headers = {
            "Authorization": f"Bearer {self.api_key}",
            "Content-Type": "application/json",
            "HTTP-Referer": "https://vael.local/playtest",
            "X-Title": "Vael Playtest",
        }
        last_error = "erro desconhecido"
        for attempt in range(self.retries + 1):
            request = urllib.request.Request(self.endpoint, data=data, headers=headers)
            try:
                with urllib.request.urlopen(request, timeout=self.timeout) as response:
                    payload = json.load(response)
                answers = payload.get("answers")
                if not isinstance(answers, dict):
                    raise DecisionError("resposta sem `answers`")
                usage = payload.get("usage") or {}
                self.last_usage = usage
                self.cost += float(usage.get("cost") or 0.0)
                self.requests += 1
                return payload
            except urllib.error.HTTPError as exc:
                detail = exc.read().decode("utf-8", "replace")[:500]
                last_error = f"HTTP {exc.code}: {detail}"
                if exc.code in (400, 401, 402, 403, 404):
                    raise DecisionError(last_error, fatal=True) from exc
                wait = 5.0
                if exc.code == 429:
                    try:
                        wait = float(exc.headers.get("Retry-After") or 5.0)
                    except (TypeError, ValueError):
                        wait = 5.0
                else:
                    wait = min(60.0, 2.0**attempt)
                time.sleep(wait)
            except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as exc:
                last_error = f"{type(exc).__name__}: {exc}"
                time.sleep(min(30.0, 2.0**attempt))
        raise DecisionError(last_error)


def _noul(answer) -> float | None:
    if not isinstance(answer, dict) or answer.get("type") != "noul":
        return None
    try:
        return float(answer.get("noul"))
    except (TypeError, ValueError):
        return None


def evaluate_answers(payload: dict, min_confidence: float) -> dict:
    """Normaliza o veredicto. Resposta em falta/refusal NUNCA é conforme."""
    answers = payload.get("answers") or {}
    visual = answers.get("visual_ok") or {}
    estado = answers.get("estado_ok") or {}
    anomaly = answers.get("anomalia") or {}
    severity = answers.get("severidade") or {}

    reasons: list[str] = []
    unanswered: list[str] = []

    visual_ok = _noul(visual)
    if visual_ok is None:
        unanswered.append("visual_ok")
        reasons.append("visual_ok não respondido")
    elif visual_ok < 0.5:
        reasons.append(f"visual_ok={visual_ok:.2f}")

    estado_ok = _noul(estado)
    if estado_ok is None:
        unanswered.append("estado_ok")
        reasons.append("estado_ok não respondido")
    elif estado_ok < 0.5:
        reasons.append(f"estado_ok={estado_ok:.2f}")

    anomaly_label = anomaly.get("choice") if anomaly.get("type") == "choice" else None
    try:
        anomaly_conf = float(anomaly.get("confidence") or 0.0)
    except (TypeError, ValueError):
        anomaly_conf = 0.0
    if anomaly_label and anomaly_label != "none" and anomaly_conf >= min_confidence:
        reasons.append(f"anomalia={anomaly_label} ({anomaly_conf:.2f})")

    severity_score = None
    if severity.get("type") == "score":
        severity_score = severity.get("score")
        try:
            severity_score = int(severity_score)
        except (TypeError, ValueError):
            pass
    severity_conf = severity.get("confidence")

    return {
        "suspect": bool(reasons),
        "reasons": reasons,
        "unanswered": unanswered,
        "visual_ok": visual_ok,
        "estado_ok": estado_ok,
        "anomalia": anomaly_label,
        "anomalia_confidence": anomaly_conf,
        "anomalia_probabilities": anomaly.get("probabilities"),
        "severidade": severity_score,
        "severidade_confidence": severity_conf,
        "answers": answers,
    }


# ── frames e estado ─────────────────────────────────────────────────────


class Frame:
    __slots__ = ("ts", "b64", "bytes")

    def __init__(self, ts: float, b64: str):
        self.ts = ts
        self.b64 = b64
        self.bytes = len(b64) * 3 // 4


class FrameBuffer:
    def __init__(self, maxlen: int = 24):
        self._frames: deque[Frame] = deque(maxlen=maxlen)
        self._lock = threading.Lock()

    def add(self, frame: Frame) -> None:
        with self._lock:
            self._frames.append(frame)

    def snapshot(self, count: int) -> list[Frame]:
        with self._lock:
            frames = list(self._frames)
        return frames[-count:]

    def __len__(self) -> int:
        with self._lock:
            return len(self._frames)


def _unlink_capture(path) -> None:
    # O store da engine não limpa o PNG do /tmp; a 10 Hz encheria o disco.
    # Só apagamos caminhos do próprio bridge.
    if isinstance(path, str) and "viber-bridge-" in path:
        try:
            os.unlink(path)
        except OSError:
            pass


def request_capture(bridge: qa.Bridge) -> tuple[int, str | None]:
    request = bridge.rpc("viber.screenshot", {}, timeout=CAPTURE_TIMEOUT_S)
    capture_id = request.get("id") if isinstance(request, dict) else None
    if capture_id is None:
        raise RuntimeError("viber.screenshot sem id")
    return int(capture_id), request.get("path")


def poll_capture(bridge: qa.Bridge, capture_id: int) -> Frame | None:
    """`None` = ainda pending; `Frame` = PNG pronto; levanta em `error`."""
    status = bridge.rpc(
        "viber.screenshot_status", {"id": capture_id}, timeout=CAPTURE_TIMEOUT_S
    )
    state = status.get("status") if isinstance(status, dict) else None
    if state == "captured":
        b64 = status.get("png_base64")
        if not b64:
            raise RuntimeError("captura sem png_base64")
        _unlink_capture(status.get("path"))
        return Frame(time.monotonic(), b64)
    if state == "error":
        raise RuntimeError(status.get("error") or "captura falhou")
    return None


def capture_png(bridge: qa.Bridge) -> Frame:
    """Captura um frame. Prefere `brp_extras/screenshot` (uma chamada, ~40 ms).

    O legado `viber.screenshot`+status continua como fallback quando os
    extras estão desligados (`VAEL_BRP_EXTRAS=0`).
    """
    global _BRP_SCREENSHOT
    if _BRP_SCREENSHOT:
        path = _CAPTURE_DIR / f"frame-{next(_CAPTURE_SEQ)}.png"
        try:
            response = bridge.rpc(
                "brp_extras/screenshot", {"path": str(path)}, timeout=10.0
            )
            if not isinstance(response, dict) or not response.get("success"):
                raise RuntimeError(f"brp_extras/screenshot: {response}")
            png = path.read_bytes()
            try:
                path.unlink()
            except OSError:
                pass
            return Frame(time.monotonic(), base64.b64encode(png).decode("ascii"))
        except Exception as exc:  # noqa: BLE001
            message = str(exc).lower()
            if "method not found" in message or "-32601" in message:
                _BRP_SCREENSHOT = False
                log(
                    "brp_extras/screenshot indisponível; fallback para "
                    "viber.screenshot (mais lento)"
                )
            else:
                raise
    # Fallback legado: pedido + polling do status.
    capture_id, _ = request_capture(bridge)
    deadline = time.monotonic() + CAPTURE_TIMEOUT_S
    while time.monotonic() < deadline:
        frame = poll_capture(bridge, capture_id)
        if frame is not None:
            return frame
        time.sleep(CAPTURE_POLL_S)
    raise RuntimeError("timeout à espera do screenshot")


class CaptureWorker(threading.Thread):
    """Captura sequencial ao ritmo de `capture_hz`.

    O `viber.screenshot` + status leva ~70 ms a 480x270; esperar por cada um
    em série é mais estável do que vários pedidos em voo no servidor do
    bridge (o HTTP handler corre no main thread do jogo).
    """

    def __init__(self, bridge: qa.Bridge, buffer: FrameBuffer, hz: float, stop: threading.Event):
        super().__init__(name="vael-capture", daemon=True)
        self.bridge = bridge
        self.buffer = buffer
        self.period = 1.0 / max(hz, 0.5)
        self.stop = stop
        self.count = 0
        self.errors = 0
        self.consecutive_errors = 0
        self.fatal: str | None = None
        self.started = time.monotonic()
        self.first_frame: float | None = None

    def _error(self, exc: Exception) -> None:
        self.errors += 1
        self.consecutive_errors += 1
        if self.errors == 1 or self.errors % 10 == 0:
            log(f"captura falhou ({self.errors}): {exc}")
        if self.consecutive_errors >= 10:
            self.fatal = str(exc)
            self.stop.set()

    def run(self) -> None:
        while not self.stop.is_set():
            started = time.monotonic()
            try:
                frame = capture_png(self.bridge)
                self.buffer.add(frame)
                self.count += 1
                self.consecutive_errors = 0
                if self.first_frame is None:
                    self.first_frame = time.monotonic()
            except Exception as exc:  # noqa: BLE001 — o loop não pode morrer
                self._error(exc)
            elapsed = time.monotonic() - started
            self.stop.wait(max(0.0, self.period - elapsed))

    def achieved_hz(self) -> float:
        # Mede a partir do 1.º frame: o arranque do render (shaders/LOD) não
        # é captura e falseava a taxa para baixo.
        start = self.first_frame or self.started
        elapsed = max(1e-6, time.monotonic() - start)
        return self.count / elapsed


STATE_LUA = """
local ok, x, y, z = viber.player_position()
return {
  snapshot = viber.vael.qa_snapshot(),
  pos = { x = x, y = y, z = z, ok = ok },
}
"""


class StateSampler(threading.Thread):
    def __init__(self, bridge: qa.Bridge, stop: threading.Event):
        super().__init__(name="vael-state", daemon=True)
        self.bridge = bridge
        self.stop = stop
        self._lock = threading.Lock()
        self.latest: dict = {}
        self.error: str | None = None
        self.errors = 0

    def run(self) -> None:
        while not self.stop.is_set():
            self.sample()
            # O estado é contexto para o modelo; 5 s chega e rouba menos
            # frames ao loop de captura do que 1-2 s.
            self.stop.wait(5.0)

    def sample(self) -> None:
        try:
            # RPC direto (sem `vael-tools debug lua`): um subprocesso por
            # segundo roubava frames ao loop de captura.
            response = self.bridge.rpc("viber.lua", {"code": STATE_LUA}, timeout=5.0)
            if not isinstance(response, dict) or not response.get("ok"):
                raise RuntimeError(
                    (response or {}).get("error") or "viber.lua devolveu erro"
                )
            state = response.get("result")
            if not isinstance(state, dict):
                return
            snapshot = state.get("snapshot")
            if isinstance(snapshot, str):
                try:
                    state["snapshot"] = json.loads(snapshot)
                except json.JSONDecodeError:
                    pass
            # `viber.scene` é método do bridge, não tabela do REPL.
            try:
                scene = self.bridge.rpc("viber.scene.current", {}, timeout=3.0)
                if isinstance(scene, dict):
                    state["scene"] = scene.get("scene")
                    state["title"] = scene.get("title")
                    state["ready"] = scene.get("ready")
                    state["loading"] = scene.get("loading")
            except Exception:  # noqa: BLE001 — a cena é contexto, não gate
                pass
            state["sampled_at"] = time.time()
            with self._lock:
                self.latest = state
            self.error = None
        except Exception as exc:  # noqa: BLE001
            self.error = str(exc)
            self.errors += 1
            if self.errors == 1 or self.errors % 10 == 0:
                log(f"estado falhou ({self.errors}): {exc}")

    def get(self) -> dict:
        with self._lock:
            return dict(self.latest)


def game_logs(bridge: qa.Bridge, limit: int = 120) -> list:
    try:
        logs = bridge.rpc("viber.logs", {"limit": limit}, timeout=5.0)
        return logs if isinstance(logs, list) else []
    except Exception:  # noqa: BLE001
        return []


# ── pedido e veredicto ───────────────────────────────────────────────────


def _image_part(b64: str) -> dict:
    return {
        "type": "image_url",
        "image_url": {"url": f"data:image/png;base64,{b64}", "detail": "low"},
    }


def build_request(
    model: str,
    context: str,
    state: dict,
    frames: list[Frame],
    baseline_b64: str | None,
    frames_per_request: int,
) -> tuple[dict, str]:
    state_text = json.dumps(state, ensure_ascii=False, default=str)
    if len(state_text) > 6000:
        state_text = state_text[:6000] + "…"
    times = ", ".join(f"{frame.ts:.2f}" for frame in frames)
    text = (
        "VAEL playtest — visual and state conformance.\n"
        f"Context: {context}\n"
        f"Frames in this window: {len(frames)} captured at ~10 Hz (monotonic timestamps: {times}).\n"
        f"State snapshot (JSON): {state_text}\n"
        "Judge the attached frames in order. The text state is context for the "
        "state question; the images are the primary evidence for the visual question."
    )
    selected = frames[-(frames_per_request - (1 if baseline_b64 else 0)) :]
    if baseline_b64:
        selected = [Frame(0.0, baseline_b64)] + selected
    selected = selected[:MAX_FRAMES]
    payload = {
        "model": model,
        "state": [text, *[_image_part(frame.b64) for frame in selected]],
        "questions": QUESTIONS,
    }
    return payload, text


def request_bytes(payload: dict) -> int:
    return len(json.dumps(payload, ensure_ascii=False).encode("utf-8"))


# ── artefactos ──────────────────────────────────────────────────────────


class Artifacts:
    def __init__(self, root: Path):
        self.root = root
        self.suspects = root / "suspects"
        self.root.mkdir(parents=True, exist_ok=True)
        self.verdicts_path = root / "verdicts.jsonl"
        self._lock = threading.Lock()
        self.suspect_count = 0
        self.decision_count = 0

    def append_verdict(self, record: dict) -> None:
        with self._lock:
            self.decision_count += 1
            with self.verdicts_path.open("a", encoding="utf-8") as handle:
                handle.write(json.dumps(record, ensure_ascii=False, default=str) + "\n")

    def save_suspect(
        self,
        frames: list[Frame],
        request_text: str,
        verdict: dict,
        payload: dict,
        state: dict,
        logs: list,
        game_log_tail: str,
    ) -> Path:
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
        directory = self.suspects / stamp
        frames_dir = directory / "frames"
        frames_dir.mkdir(parents=True, exist_ok=True)
        frame_meta = []
        for index, frame in enumerate(frames):
            name = f"frame-{index:03d}.png"
            (frames_dir / name).write_bytes(base64.b64decode(frame.b64))
            frame_meta.append({"file": name, "ts": frame.ts, "bytes": frame.bytes})
        self._write_json(
            directory / "verdict.json",
            {
                "verdict": verdict,
                "usage": payload.get("usage"),
                "model": payload.get("model"),
                "answers": payload.get("answers"),
            },
        )
        self._write_json(
            directory / "request.json",
            {
                "text": request_text,
                "frames": frame_meta,
                "questions": QUESTIONS,
                "model": payload.get("model"),
                "body_bytes": request_bytes(payload),
            },
        )
        self._write_json(directory / "state.json", state)
        self._write_json(directory / "logs.json", logs)
        (directory / "game.log").write_text(game_log_tail, encoding="utf-8")
        reasons = "\n".join(f"- {reason}" for reason in verdict["reasons"]) or "- (sem motivo)"
        report = (
            f"# Playtest visual — suspeita {stamp}\n\n"
            f"## Motivos\n{reasons}\n\n"
            f"## Respostas\n```json\n"
            f"{json.dumps(payload.get('answers') or {}, ensure_ascii=False, indent=2)}\n```\n\n"
            f"## Custo do pedido\n{json.dumps(payload.get('usage') or {}, ensure_ascii=False)}\n\n"
            f"## Frames\n{len(frames)} em `frames/`\n\n"
            f"## Reproduzir\n```bash\n"
            f"python3 tools/playtest_vision.py --tour <tour> --duration 90\n```\n"
        )
        (directory / "report.md").write_text(report, encoding="utf-8")
        with self._lock:
            self.suspect_count += 1
        return directory

    @staticmethod
    def _write_json(path: Path, value) -> None:
        path.write_text(
            json.dumps(value, ensure_ascii=False, indent=2, default=str) + "\n",
            encoding="utf-8",
        )

    def summary(self) -> dict:
        return {
            "dir": str(self.root),
            "decisions": self.decision_count,
            "suspects": self.suspect_count,
        }


# ── worker de decisão ───────────────────────────────────────────────────


class DecisionWorker(threading.Thread):
    def __init__(
        self,
        args,
        bridge: qa.Bridge,
        buffer: FrameBuffer,
        sampler: StateSampler,
        artifacts: Artifacts,
        stop: threading.Event,
        context: str,
        baseline_b64: str | None,
        client: DecisionClient | None,
        game_log_path: Path | None,
    ):
        super().__init__(name="vael-decisions", daemon=True)
        self.args = args
        self.bridge = bridge
        self.buffer = buffer
        self.sampler = sampler
        self.artifacts = artifacts
        self.stop = stop
        self.context = context
        self.baseline_b64 = baseline_b64
        self.client = client
        self.game_log_path = game_log_path
        self.period = 1.0 / max(args.decision_hz, 0.1)
        self.decisions = 0
        self.errors = 0
        self.suspects = 0
        self.fatal: str | None = None
        self.dry_run_written = 0

    def run(self) -> None:
        while not self.stop.wait(self.period):
            if self.args.max_decisions and self.decisions >= self.args.max_decisions:
                log(f"limite de {self.args.max_decisions} decisões atingido; parar")
                self.stop.set()
                return
            if self.client is not None and self.client.cost >= self.args.budget_usd:
                log(f"orçamento esgotado (${self.client.cost:.4f} >= ${self.args.budget_usd:.2f}); parar")
                self.stop.set()
                return
            frames = self.buffer.snapshot(self.args.frames_per_request)
            if len(frames) < 2:
                continue
            state = self.sampler.get()
            payload, text = build_request(
                self.args.model,
                self.context,
                state,
                frames,
                self.baseline_b64,
                self.args.frames_per_request,
            )
            # O Liquid aceita até 4,5 MB; margem para o JSON/base64.
            limit = int(self.args.max_body_mb * 1024 * 1024)
            while request_bytes(payload) > limit and len(frames) > 2:
                frames = frames[1:]
                payload, text = build_request(
                    self.args.model,
                    self.context,
                    state,
                    frames,
                    self.baseline_b64,
                    self.args.frames_per_request,
                )
            if request_bytes(payload) > limit:
                log(
                    f"pedido {request_bytes(payload)} B acima do limite "
                    f"{limit} B; saltar decisão"
                )
                continue
            if self.args.dry_run:
                self._dry_run(frames, payload, text, state)
                continue
            try:
                response = self.client.ask(
                    payload["state"],
                    payload["questions"],
                    session_id=self.args.session_id,
                )
            except DecisionError as exc:
                self.errors += 1
                log(f"decisão falhou: {exc}")
                if exc.fatal:
                    self.fatal = str(exc)
                    self.stop.set()
                    return
                continue
            verdict = evaluate_answers(response, self.args.min_confidence)
            self.decisions += 1
            self.artifacts.append_verdict(
                {
                    "ts": time.time(),
                    "verdict": verdict,
                    "usage": response.get("usage"),
                    "model": response.get("model"),
                    "scene": (state or {}).get("scene"),
                    "pos": (state or {}).get("pos"),
                    "frames": len(frames),
                }
            )
            status = "SUSPEITO" if verdict["suspect"] else "ok"
            log(
                f"decisão {self.decisions}: {status} "
                f"visual={verdict['visual_ok']} estado={verdict['estado_ok']} "
                f"anomalia={verdict['anomalia']} severidade={verdict['severidade']} "
                f"custo=${self.client.cost:.4f}"
            )
            if not verdict["suspect"]:
                continue
            self.suspects += 1
            logs = game_logs(self.bridge)
            log_tail = ""
            if self.game_log_path and self.game_log_path.is_file():
                try:
                    log_tail = self.game_log_path.read_text(
                        encoding="utf-8", errors="replace"
                    )[-4000:]
                except OSError:
                    log_tail = ""
            directory = self.artifacts.save_suspect(
                frames, text, verdict, response, state, logs, log_tail
            )
            log(f"artefacto em {directory}")
            if self.args.pause_on_anomaly:
                try:
                    self.bridge.rpc("vael-tools/pause", {}, timeout=5.0)
                    log("jogo pausado para inspeção")
                except Exception as exc:  # noqa: BLE001
                    log(f"pause falhou: {exc}")
            if self.args.stop_on_anomaly:
                self.stop.set()
                return

    def _dry_run(self, frames: list[Frame], payload: dict, text: str, state: dict) -> None:
        self.decisions += 1
        if self.dry_run_written >= 3:
            log(f"dry-run: decisão {self.decisions} (pedido omitido após 3 amostras)")
            return
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
        directory = self.artifacts.root / "dry-run" / stamp
        frames_dir = directory / "frames"
        frames_dir.mkdir(parents=True, exist_ok=True)
        meta = []
        for index, frame in enumerate(frames):
            name = f"frame-{index:03d}.png"
            (frames_dir / name).write_bytes(base64.b64decode(frame.b64))
            meta.append({"file": name, "ts": frame.ts, "bytes": frame.bytes})
        self.artifacts._write_json(
            directory / "request.json",
            {
                "text": text,
                "frames": meta,
                "questions": QUESTIONS,
                "model": payload.get("model"),
                "body_bytes": request_bytes(payload),
            },
        )
        self.artifacts._write_json(directory / "state.json", state)
        self.dry_run_written += 1
        log(
            f"dry-run: pedido {self.dry_run_written} em {directory} "
            f"({request_bytes(payload)} B, {len(frames)} frames)"
        )


# ── jogo e tours ────────────────────────────────────────────────────────


def launch_game(args, qa_scenario: str | None, artifacts: Artifacts):
    if qa.Bridge(args.port).listening():
        die(f"já há um jogo na porta {args.port} (usa --attach ou --port)")
    log_path = artifacts.root / "game.log"
    if not args.no_build:
        log("a compilar o jogo (cargo build --release)…")
        if not args.debug:
            qa.build_game(release=True)
        else:
            qa.build_game(release=False)
    profile = "debug" if args.debug else "release"
    exe = ROOT / "target" / profile / "vael"
    if not exe.is_file():
        die(f"binário não existe: {exe} (corre sem --no-build)")
    cmd = [
        str(exe),
        "world.bsn",
        "--bridge",
        str(args.port),
        "--resolution",
        args.resolution,
    ]
    if qa_scenario:
        cmd += ["--scenario", qa_scenario]
    log_file = log_path.open("wb")
    log("a lançar: " + " ".join(cmd))
    # O pacer adaptativo vê o custo do próprio screenshot e trava o jogo
    # num divisor baixo (30 fps), atrasando a captura seguinte em cadeia.
    # Para o oráculo visual o jogo corre sem pacing.
    env = os.environ.copy()
    env["VAEL_FPS_CAP"] = "0"
    # Nunca tocar nos saves reais (o menu não tem cenário QA).
    saves = artifacts.root / "saves"
    saves.mkdir(parents=True, exist_ok=True)
    env["VAEL_QA_SAVE_DIR"] = str(saves)
    process = subprocess.Popen(
        cmd,
        cwd=ROOT,
        stdout=log_file,
        stderr=subprocess.STDOUT,
        start_new_session=True,
        env=env,
    )
    return process, log_path, log_file


def stop_game(process: subprocess.Popen | None, keep: bool) -> None:
    if process is None or keep or process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except OSError:
        process.terminate()
    try:
        process.wait(timeout=15)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except OSError:
            process.kill()


def wait_scene_ready(bridge: qa.Bridge, timeout: float = 45.0) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            current = bridge.rpc("viber.scene.current", {}, timeout=5.0)
            if isinstance(current, dict) and current.get("ready") and not current.get(
                "loading"
            ):
                return True
        except Exception:  # noqa: BLE001 — a troca de cena ainda está a correr
            pass
        time.sleep(0.5)
    return False


def run_tour_step(bridge: qa.Bridge, step: dict, shots: Path) -> str:
    if "scene" in step:
        name = step["scene"]
        params = {"scene": name}
        if step.get("entry"):
            params["entry"] = step["entry"]
        deadline = time.monotonic() + 20.0
        last_error = None
        while time.monotonic() < deadline:
            try:
                bridge.rpc("viber.scene.change", params, timeout=10.0)
                break
            except Exception as exc:  # noqa: BLE001 — troca em curso
                last_error = exc
                time.sleep(0.5)
        else:
            raise RuntimeError(f"troca para {name!r} falhou: {last_error}")
        if not wait_scene_ready(bridge, step.get("timeout", 45.0)):
            raise RuntimeError(f"cena {name!r} não ficou ready")
        time.sleep(float(step.get("wait", 1.0)))
        return f"scene {name}"
    return qa.live_step(bridge, step, shots)


def run_tour_steps(
    bridge: qa.Bridge,
    steps: list[dict],
    shots: Path,
    continue_on_error: bool,
) -> list[str]:
    errors = []
    for index, step in enumerate(steps, 1):
        label = next(iter(step)) if isinstance(step, dict) and step else "?"
        try:
            detail = run_tour_step(bridge, step, shots)
            log(f"passo {index} {label}: {detail}")
        except Exception as exc:  # noqa: BLE001
            message = f"passo {index} {label}: {exc}"
            log("FALHA " + message)
            errors.append(message)
            if not continue_on_error:
                break
    return errors


# ── orquestração ────────────────────────────────────────────────────────


def load_tour(path: str) -> dict:
    tour_path = Path(path)
    if not tour_path.is_absolute():
        tour_path = ROOT / tour_path
    try:
        tour = json.loads(tour_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        die(f"tour {path}: {exc}")
    if not isinstance(tour, dict):
        die(f"tour {path}: o topo tem de ser um objeto")
    return tour


def slug(text: str) -> str:
    return "".join(ch if ch.isalnum() or ch in "-_" else "-" for ch in text).strip("-")[:48]


def run(args) -> int:
    tour = load_tour(args.tour) if args.tour else {}
    qa_scenario = args.scenario or tour.get("qa_scenario")
    context = args.context or tour.get("context") or "Vael game scene"
    steps = tour.get("steps") or []
    baseline = bool(tour.get("baseline", False))
    run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    label = (
        args.tour_name
        or tour.get("name")
        or args.scenario
        or (Path(args.tour).stem if args.tour else None)
        or "watch"
    )
    artifacts = Artifacts(ROOT / "qa" / "playtest-artifacts" / f"{run_id}-{slug(str(label))}")
    shots = artifacts.root / "shots"
    shots.mkdir(parents=True, exist_ok=True)

    key = None if args.dry_run else load_secrets(args.secrets)
    # `secrets.env` pode trazer o modelo/endpoint; o ambiente ganha.
    args.model = os.environ.get("OPENROUTER_DECISION_MODEL", args.model)
    args.endpoint = os.environ.get("OPENROUTER_DECISIONS_URL", args.endpoint)
    client = (
        None
        if args.dry_run
        else DecisionClient(
            key,
            model=args.model,
            endpoint=args.endpoint,
            timeout=args.timeout,
            retries=args.retries,
        )
    )
    args.session_id = f"vael-{run_id}-{slug(str(label))}"

    process = None
    log_file = None
    game_log_path: Path | None = None
    if args.attach:
        bridge = qa.Bridge(args.attach)
        if not bridge.listening():
            die(f"não há jogo na porta {args.attach}")
        if not bridge.wait_ready(args.ready_timeout):
            die(f"jogo na porta {args.attach} não ficou ready")
        port = args.attach
        log(f"a observar o jogo na porta {port}")
    else:
        process, game_log_path, log_file = launch_game(args, qa_scenario, artifacts)
        port = args.port
        bridge = qa.Bridge(port)
        if not bridge.wait_ready(args.ready_timeout):
            stop_game(process, keep=False)
            _print_log_tail(game_log_path)
            die("jogo sem `ready`; vê o game.log")
        log(f"jogo ready na porta {port}")
        time.sleep(1.0)

    stop = threading.Event()
    buffer = FrameBuffer(maxlen=max(24, args.frames_per_request * 2))
    sampler = StateSampler(bridge, stop)
    capture = CaptureWorker(bridge, buffer, args.capture_hz, stop)
    decision = DecisionWorker(
        args,
        bridge,
        buffer,
        sampler,
        artifacts,
        stop,
        context,
        None,
        client,
        game_log_path,
    )

    sampler.start()
    baseline_b64 = None
    if baseline:
        # Uma captura de referência antes de o loop de captura arrancar (o
        # render só aceita uma `Screenshot` por janela/frame).
        try:
            baseline_frame = capture_png(bridge)
            baseline_b64 = baseline_frame.b64
            (artifacts.root / "baseline.png").write_bytes(
                base64.b64decode(baseline_frame.b64)
            )
            decision.baseline_b64 = baseline_b64
            log("baseline capturado")
        except Exception as exc:  # noqa: BLE001
            log(f"baseline falhou: {exc}")
    capture.start()
    decision.start()

    step_errors: list[str] = []
    game_exited = False
    started = time.monotonic()
    try:
        if steps:
            step_errors = run_tour_steps(
                bridge, steps, shots, args.continue_on_step_error
            )
        duration = args.duration
        if duration <= 0:
            time.sleep(3.0)
        else:
            while time.monotonic() - started < duration:
                if stop.is_set():
                    break
                if process is not None and process.poll() is not None:
                    log(f"o jogo saiu (código {process.returncode})")
                    game_exited = True
                    break
                if capture.fatal:
                    log(f"captura abortou: {capture.fatal}")
                    break
                time.sleep(0.2)
    except KeyboardInterrupt:
        log("interrompido")
    finally:
        stop.set()
        decision.join(timeout=90)
        capture.join(timeout=10)
        sampler.join(timeout=10)
        if log_file is not None:
            log_file.flush()

    summary = {
        "run": str(artifacts.root),
        "tour": label,
        "qa_scenario": qa_scenario,
        "steps": len(steps),
        "step_errors": step_errors,
        "capture": {
            "frames": capture.count,
            "errors": capture.errors,
            "achieved_hz": round(capture.achieved_hz(), 2),
            "warmup_s": (
                round(capture.first_frame - capture.started, 2)
                if capture.first_frame
                else None
            ),
            "fatal": capture.fatal,
        },
        "decisions": decision.decisions,
        "decision_errors": decision.errors,
        "decision_fatal": decision.fatal,
        "suspects": decision.suspects,
        "game_exited": game_exited,
        "cost_usd": round(client.cost, 6) if client else 0.0,
        "budget_usd": args.budget_usd,
        "artifacts": artifacts.summary(),
        "dry_run": bool(args.dry_run),
    }
    (artifacts.root / "summary.json").write_text(
        json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    _write_report(artifacts.root, summary)

    if process is not None:
        stop_game(process, keep=args.keep)
        if args.keep:
            log(f"jogo deixado a correr (pid {process.pid}, porta {port})")

    if args.json:
        print(json.dumps(summary, ensure_ascii=False))
    else:
        log(
            f"fim: {summary['capture']['frames']} frames "
            f"({summary['capture']['achieved_hz']} Hz), "
            f"{summary['decisions']} decisões, {summary['suspects']} suspeitas, "
            f"${summary['cost_usd']:.4f}; artefactos em {artifacts.root}"
        )
    if summary["capture"]["fatal"] or summary["decision_fatal"] or game_exited:
        return 1
    if args.fail_on_anomaly and summary["suspects"] > 0:
        return 1
    if summary["step_errors"] and not args.continue_on_step_error:
        return 1
    return 0


def _write_report(root: Path, summary: dict) -> None:
    lines = [
        f"# Playtest visual — {summary['tour']}",
        "",
        f"- Cenário QA: `{summary['qa_scenario']}`",
        f"- Frames capturados: {summary['capture']['frames']} "
        f"(~{summary['capture']['achieved_hz']} Hz; erros {summary['capture']['errors']})",
        f"- Decisões: {summary['decisions']} (erros {summary['decision_errors']})",
        f"- Suspeitas: {summary['suspects']}",
        f"- Custo: ${summary['cost_usd']:.4f} (orçamento ${summary['budget_usd']:.2f})",
        f"- Artefactos: `{summary['artifacts']['dir']}`",
        f"- Dry-run: {summary['dry_run']}",
    ]
    if summary["step_errors"]:
        lines += ["", "## Falhas de passos"] + [
            f"- {error}" for error in summary["step_errors"]
        ]
    lines += [
        "",
        "## Reproduzir",
        "```bash",
        "python3 tools/playtest_vision.py "
        f"--scenario {summary['qa_scenario'] or '<start-scene>'} --duration 90",
        "```",
    ]
    (root / "report.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def _print_log_tail(path: Path | None) -> None:
    if not path or not path.is_file():
        return
    try:
        tail = path.read_text(encoding="utf-8", errors="replace")[-4000:]
        log("últimas linhas do game.log:\n" + tail)
    except OSError:
        pass


# ── probe / self-test ───────────────────────────────────────────────────


def probe(args) -> int:
    load_secrets(args.secrets)
    args.model = os.environ.get("OPENROUTER_DECISION_MODEL", args.model)
    args.endpoint = os.environ.get("OPENROUTER_DECISIONS_URL", args.endpoint)
    client = DecisionClient(
        os.environ["OPENROUTER_API_KEY"],
        model=args.model,
        endpoint=args.endpoint,
        timeout=args.timeout,
        retries=args.retries,
    )
    payload = client.ask(
        ["Vael playtest probe: a sunny meadow with a castle and a third-person hero."],
        {
            "is_game_scene": {
                "type": "noul",
                "instructions": "Does this text describe a video game scene?",
                "criteria": {
                    "true": "It describes a scene in a video game.",
                    "false": "It does not describe a game scene.",
                },
            }
        },
        session_id="vael-probe",
    )
    result = {
        "model": payload.get("model"),
        "answers": payload.get("answers"),
        "usage": payload.get("usage"),
        "endpoint": args.endpoint,
    }
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0


def self_test() -> int:
    good = {
        "answers": {
            "visual_ok": {"type": "noul", "noul": 0.91},
            "estado_ok": {"type": "noul", "noul": 0.88},
            "anomalia": {"type": "choice", "choice": "none", "confidence": 0.93},
            "severidade": {"type": "score", "score": 0, "confidence": 0.9},
        }
    }
    verdict = evaluate_answers(good, DEFAULT_MIN_CONFIDENCE)
    assert not verdict["suspect"], verdict

    bad = json.loads(json.dumps(good))
    bad["answers"]["visual_ok"]["noul"] = 0.2
    assert evaluate_answers(bad, DEFAULT_MIN_CONFIDENCE)["suspect"]

    anomaly = json.loads(json.dumps(good))
    anomaly["answers"]["anomalia"] = {
        "type": "choice",
        "choice": "missing_texture",
        "confidence": 0.8,
    }
    assert evaluate_answers(anomaly, DEFAULT_MIN_CONFIDENCE)["suspect"]

    refusal = {"answers": {"visual_ok": {"type": "refusal"}}}
    refused = evaluate_answers(refusal, DEFAULT_MIN_CONFIDENCE)
    assert refused["suspect"] and "visual_ok" in refused["unanswered"], refused

    log("self-test ok")
    return 0


# ── CLI ─────────────────────────────────────────────────────────────────


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Oráculo visual+estado do Vael (OpenRouter Decisions / liquid/d1-ultra)."
    )
    parser.add_argument("--scenario", help="cenário QA a lançar (ex.: combat/arena)")
    parser.add_argument("--tour", help="JSON do tour (qa/vision/tours/*.json)")
    parser.add_argument("--tour-name", help="rótulo nos artefactos")
    parser.add_argument("--context", help="contexto textual extra para o modelo")
    parser.add_argument("--attach", type=int, help="observar um jogo já aberto nesta porta")
    parser.add_argument("--port", type=int, default=qa.DEFAULT_PORT, help="porta da bridge")
    parser.add_argument("--resolution", default=DEFAULT_RESOLUTION, help="WxH da janela")
    parser.add_argument("--no-build", action="store_true", help="não compilar o jogo")
    parser.add_argument("--debug", action="store_true", help="usar target/debug")
    parser.add_argument("--duration", type=float, default=DEFAULT_DURATION,
                        help="segundos de observação (0 = até aos passos + 3 s)")
    parser.add_argument("--capture-hz", type=float, default=DEFAULT_CAPTURE_HZ)
    parser.add_argument("--decision-hz", type=float, default=DEFAULT_DECISION_HZ)
    parser.add_argument("--frames-per-request", type=int, default=DEFAULT_FRAMES)
    parser.add_argument("--budget-usd", type=float, default=DEFAULT_BUDGET_USD)
    parser.add_argument("--max-decisions", type=int, default=0)
    parser.add_argument("--min-confidence", type=float, default=DEFAULT_MIN_CONFIDENCE)
    parser.add_argument("--max-body-mb", type=float, default=DEFAULT_MAX_BODY_MB)
    parser.add_argument("--pause-on-anomaly", action="store_true")
    parser.add_argument("--stop-on-anomaly", action="store_true")
    parser.add_argument("--fail-on-anomaly", action="store_true")
    parser.add_argument("--continue-on-step-error", action="store_true")
    parser.add_argument("--keep", action="store_true", help="deixar o jogo a correr")
    parser.add_argument("--dry-run", action="store_true", help="não chamar a API")
    parser.add_argument("--probe", action="store_true", help="pedido mínimo à API e sai")
    parser.add_argument("--self-test", action="store_true", help="testa parsing/limiares e sai")
    parser.add_argument("--json", action="store_true", help="resumo final em JSON")
    parser.add_argument("--secrets", help="ficheiro alternativo a secrets.env")
    parser.add_argument("--model", default=DEFAULT_MODEL)
    parser.add_argument("--endpoint", default=DEFAULT_ENDPOINT)
    parser.add_argument("--timeout", type=float, default=60.0)
    parser.add_argument("--retries", type=int, default=3)
    parser.add_argument("--ready-timeout", type=float, default=300.0)
    args = parser.parse_args()

    args.frames_per_request = max(2, min(MAX_FRAMES, args.frames_per_request))
    args.capture_hz = max(0.5, args.capture_hz)
    args.decision_hz = max(0.1, min(args.decision_hz, args.capture_hz))
    args.budget_usd = max(0.0, args.budget_usd)
    args.max_body_mb = max(0.5, min(args.max_body_mb, 4.5))
    args.min_confidence = max(0.0, min(args.min_confidence, 1.0))

    if args.self_test:
        return self_test()
    if args.probe:
        return probe(args)
    return run(args)


if __name__ == "__main__":
    raise SystemExit(main())
