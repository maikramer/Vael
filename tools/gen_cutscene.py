#!/usr/bin/env python3
"""Gera vinhetas de cutscenes em mangá via `text2d` e compõe previews de pranchas.

Lê `data/cutscenes/<id>.json`, gera cada quadro em `assets/public/<panel.image>`
com o estilo canónico de mangá do Vael (`docs/ART.md` §9) e produz folhas de
verificação (`assets/public/cutscenes/<id>/_preview_<page>.png`) com os cortes
diagonais, calhas, bolhas e onomatopeias exactos do JSON.

Uso:
    python3 tools/gen_cutscene.py prologo
    python3 tools/gen_cutscene.py prologo --only p1_discordia --force
    python3 tools/gen_cutscene.py prologo --preview-only
    python3 tools/gen_cutscene.py prologo --dry-run
"""

from __future__ import annotations

import argparse
import json
import math
import os
import subprocess
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
CUTSCENES_DIR = ROOT / "data" / "cutscenes"
ASSETS_PUBLIC = ROOT / "assets" / "public"
FONT_BOLD = ROOT / "ui" / "fonts" / "NotoSerifDisplay-Bold.ttf"
FONT_REG = ROOT / "ui" / "fonts" / "NotoSerifDisplay-Regular.ttf"

STYLE_PREFIX = (
    "hand-inked black and white manga comic illustration, thick crisp pen contour lines, "
    "bold solid black ink shadow blocks, expressive fine ink crosshatching, "
    "rural fantasy adults at 6.5 heads proportions with sturdy expressive hands, "
    "chamfered timber and limewashed stone architecture, matte paper ink look, "
    "no screentone dots, no anime sparkle, no chibi, no speech bubbles, no text, no watermark, "
    "full bleed composition with no border frame"
)

ACCENT_HINTS = {
    "amber": "strictly monochrome black and white ink except one single warm forge-amber orange accent pigment on light sources",
    "turquoise": "strictly monochrome black and white ink except one single cool choir-turquoise blue accent pigment on crystal or water glow",
    "none": "pure monochrome black ink on warm off-white paper, zero color accents",
}


def build_prompt(panel: dict[str, Any]) -> str:
    action = (panel.get("prompt") or "").strip()
    accent = str(panel.get("accent") or "none").lower()
    accent_hint = ACCENT_HINTS.get(accent, ACCENT_HINTS["none"])
    return f"{STYLE_PREFIX}, {accent_hint}. Scene: {action}"


def round16(value: float, lo: int = 448, hi: int = 1280) -> int:
    clamped = max(lo, min(hi, int(round(value))))
    return max(lo, (clamped // 16) * 16)


def infer_dimensions(panel: dict[str, Any]) -> tuple[int, int]:
    if panel.get("width") and panel.get("height"):
        return int(panel["width"]), int(panel["height"])
    xs = [p[0] for p in panel["quad"]]
    ys = [p[1] for p in panel["quad"]]
    w_px = (max(xs) - min(xs)) * 16.0
    h_px = (max(ys) - min(ys)) * 9.0
    aspect = max(0.5, min(3.2, w_px / max(h_px, 0.1)))
    if aspect >= 1.0:
        w = 1024 if aspect >= 1.7 else 768
        h = round16(w / aspect, lo=384, hi=768)
    else:
        h = 768
        w = round16(h * aspect, lo=384, hi=768)
    return w, h


def generate_panel(
    panel: dict[str, Any],
    *,
    force: bool,
    dry_run: bool,
    steps: int | None,
    gpu: str,
    no_vramd: bool,
    cpu: bool,
) -> Path:
    out_path = ASSETS_PUBLIC / panel["image"]
    out_path.parent.mkdir(parents=True, exist_ok=True)
    if out_path.exists() and not force:
        print(f"[skip] {panel['id']} -> {out_path.relative_to(ROOT)} (já existe; usa --force)")
        return out_path

    prompt = build_prompt(panel)
    w, h = infer_dimensions(panel)
    cmd = [
        "text2d",
        "generate",
        prompt,
        "-o",
        str(out_path),
        "-W",
        str(w),
        "-H",
        str(h),
    ]
    if steps is not None:
        cmd += ["-s", str(steps)]
    if panel.get("seed") is not None:
        cmd += ["--seed", str(int(panel["seed"]))]
    if panel.get("reference"):
        ref_path = ASSETS_PUBLIC / panel["reference"]
        if ref_path.exists():
            cmd += ["--ref", str(ref_path)]
    if gpu and gpu != "auto":
        cmd += ["--gpu", gpu]
    if no_vramd:
        cmd += ["--no-vramd"]
    if cpu:
        cmd += ["--cpu"]

    print(f"[gen]  {panel['id']} ({w}x{h}) -> {out_path.relative_to(ROOT)}")
    if dry_run:
        print("       ", " ".join(cmd[:6]), "...")
        return out_path

    env = os.environ.copy()
    if gpu == "local":
        env.pop("VRAMD_REMOTE", None)
        env.pop("VRAMD_REMOTE_TOKEN", None)
    res = subprocess.run(cmd, check=False, env=env)
    if res.returncode != 0:
        raise RuntimeError(f"text2d falhou em {panel['id']} (exit {res.returncode})")
    return out_path


# ── Compositor de preview das pranchas (Pillow) ─────────────────────────


def _paper_rgb(paper: str) -> tuple[int, int, int]:
    return {
        "parchment": (234, 224, 201),
        "night": (19, 18, 22),
        "charcoal": (37, 34, 33),
    }.get(paper, (234, 224, 201))


def _ink_rgb(paper: str) -> tuple[int, int, int]:
    return (224, 217, 201) if paper == "night" else (20, 18, 16)


def _accent_rgb(accent: str) -> tuple[int, int, int]:
    return {
        "amber": (224, 138, 56),
        "turquoise": (74, 184, 199),
        "none": (138, 122, 97),
    }.get(accent, (138, 122, 97))


def _wrap_text(draw: Any, text: str, font: Any, max_w: int) -> list[str]:
    words = text.split()
    lines: list[str] = []
    cur = ""
    for w in words:
        trial = f"{cur} {w}".strip()
        if draw.textlength(trial, font=font) <= max_w or not cur:
            cur = trial
        else:
            lines.append(cur)
            cur = w
    if cur:
        lines.append(cur)
    return lines


def render_page_preview(spec: dict[str, Any], page: dict[str, Any], out_dir: Path) -> Path:
    from PIL import Image, ImageDraw, ImageFont

    W, H = 1920, 1080
    paper = page.get("paper", "parchment")
    canvas = Image.new("RGBA", (W, H), (*_paper_rgb(paper), 255))
    ink = (*_ink_rgb(paper), 255)

    def load_font(path: Path, size: int) -> Any:
        if path.exists():
            return ImageFont.truetype(str(path), size)
        return ImageFont.load_default()

    # 1. Quadros diagonais
    for panel in page["panels"]:
        poly = [(int(x * W), int(y * H)) for x, y in panel["quad"]]
        xs = [p[0] for p in poly]
        ys = [p[1] for p in poly]
        x0, y0, x1, y1 = min(xs), min(ys), max(xs), max(ys)
        bw, bh = max(1, x1 - x0), max(1, y1 - y0)

        # Sombra na calha
        shadow = Image.new("RGBA", (W, H), (0, 0, 0, 0))
        sdraw = ImageDraw.Draw(shadow)
        sdraw.polygon([(px + 6, py + 8) for px, py in poly], fill=(10, 9, 8, 65))
        canvas = Image.alpha_composite(canvas, shadow)

        img_path = ASSETS_PUBLIC / panel["image"]
        if img_path.exists():
            art = Image.open(img_path).convert("RGBA")
            # Cover crop + pan_to/zoom_to idênticos ao shader manga_panel.wesl
            pan = panel.get("pan_to", [0.0, 0.0])
            zoom = max(0.5, float(panel.get("zoom_to", 1.0)))
            box_aspect = bw / max(bh, 1.0)
            tex_aspect = art.width / max(art.height, 1.0)
            fit_x, fit_y = 1.0, 1.0
            if box_aspect > tex_aspect:
                fit_y = tex_aspect / box_aspect
            else:
                fit_x = box_aspect / tex_aspect
            span_u = min(1.0, fit_x / zoom)
            span_v = min(1.0, fit_y / zoom)
            u0 = max(0.0, min(1.0 - span_u, 0.5 + float(pan[0]) - span_u * 0.5))
            v0 = max(0.0, min(1.0 - span_v, 0.5 + float(pan[1]) - span_v * 0.5))
            crop_box = (
                int(round(u0 * art.width)),
                int(round(v0 * art.height)),
                max(1, int(round((u0 + span_u) * art.width))),
                max(1, int(round((v0 + span_v) * art.height))),
            )
            art = art.crop(crop_box).resize((bw, bh), Image.Resampling.LANCZOS)
        else:
            art = Image.new("RGBA", (bw, bh), (222, 212, 190, 255))
            adraw = ImageDraw.Draw(art)
            for k in range(-bh, bw, 28):
                adraw.line([(k, 0), (k + bh, bh)], fill=(180, 168, 146, 160), width=2)
            adraw.text((24, 24), f"[sem arte: {panel['id']}]", fill=(40, 35, 30, 255), font=load_font(FONT_BOLD, 28))

        layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
        layer.paste(art, (x0, y0))
        mask = Image.new("L", (W, H), 0)
        ImageDraw.Draw(mask).polygon(poly, fill=255)
        canvas.paste(layer, (0, 0), mask)

        # Contorno grosso de tinta
        draw = ImageDraw.Draw(canvas)
        border_w = max(4, int(round(panel.get("border_px", 5.0) * 1.4)))
        for i in range(4):
            draw.line([poly[i], poly[(i + 1) % 4]], fill=ink, width=border_w)

    draw = ImageDraw.Draw(canvas)

    # 2. Título opcional
    if page.get("title_card"):
        tc = page["title_card"]
        tx, ty = int(tc.get("pos", [0.05, 0.05])[0] * W), int(tc.get("pos", [0.05, 0.05])[1] * H)
        f_sub = load_font(FONT_REG, 22)
        f_title = load_font(FONT_BOLD, 46)
        tw = max(draw.textlength(tc["text"], font=f_title), draw.textlength(tc.get("sub", ""), font=f_sub)) + 56
        th = 112 if tc.get("sub") else 78
        draw.rectangle([tx, ty, tx + tw, ty + th], fill=(19, 18, 16, 236), outline=(235, 222, 194, 240), width=4)
        cy = ty + 14
        if tc.get("sub"):
            draw.text((tx + 28, cy), tc["sub"], fill=(224, 184, 112, 255), font=f_sub)
            cy += 30
        draw.text((tx + 28, cy), tc["text"], fill=(246, 238, 220, 255), font=f_title)

    # 3. Bolhas e onomatopeias
    for b in page.get("bubbles", []):
        kind = b.get("kind", "speech")
        bx = int(b["pos"][0] * W)
        by = int(b["pos"][1] * H)
        bw = int(b.get("width", 0.28) * W)
        accent_c = (*_accent_rgb(b.get("accent", "none")), 255)

        if kind == "sfx":
            f_sfx = load_font(FONT_BOLD, int(b.get("size", 34) * 1.55))
            txt_img = Image.new("RGBA", (600, 180), (0, 0, 0, 0))
            tdraw = ImageDraw.Draw(txt_img)
            for ox, oy in [(-4, -4), (4, 4), (4, -3), (-3, 4)]:
                tdraw.text((40 + ox, 40 + oy), b["text"], fill=(15, 14, 12, 255), font=f_sfx)
            col = (245, 236, 216, 255) if b.get("accent", "none") == "none" else accent_c
            tdraw.text((40, 40), b["text"], fill=col, font=f_sfx)
            rot = txt_img.rotate(-float(b.get("angle_deg", 0.0)), resample=Image.Resampling.BICUBIC, expand=True)
            canvas.alpha_composite(rot, (bx - 20, by - 20))
            draw = ImageDraw.Draw(canvas)
            continue

        f_sp = load_font(FONT_BOLD, 18)
        f_body = load_font(FONT_BOLD if kind == "shout" else FONT_REG, 26 if kind == "shout" else 23)
        pad_x, pad_y = (36, 26) if kind in ("speech", "shout") else (24, 18)
        lines = _wrap_text(draw, b["text"], f_body, max(120, bw - pad_x * 2))
        speaker = (b.get("speaker") or "").strip().upper()
        bh = pad_y * 2 + (24 if speaker else 0) + len(lines) * 32

        if kind == "narration":
            draw.rectangle([bx + 4, by + 5, bx + bw + 4, by + bh + 5], fill=(12, 11, 10, 70))
            draw.rectangle([bx, by, bx + bw, by + bh], fill=(239, 229, 206, 245), outline=(20, 18, 16, 255), width=4)
            draw.rectangle([bx + 4, by + 4, bx + 13, by + bh - 4], fill=accent_c)
        elif kind == "shout":
            cx, cy = bx + bw / 2, by + bh / 2
            rx, ry = bw / 2 + 12, bh / 2 + 14
            pts = []
            for i in range(32):
                ang = i * math.tau / 32.0
                r = 1.24 if i % 2 == 0 else 0.96
                pts.append((cx + math.cos(ang) * rx * r, cy + math.sin(ang) * ry * r))
            draw.polygon(pts, fill=(252, 248, 236, 252), outline=(18, 16, 14, 255), width=5)
        else:
            if kind == "speech" and b.get("tail"):
                tx, ty = int(b["tail"][0] * W), int(b["tail"][1] * H)
                cx, cy = bx + bw // 2, by + bh // 2
                dx, dy = tx - cx, ty - cy
                ln = max(1.0, math.hypot(dx, dy))
                px, py = -dy / ln * 18, dx / ln * 18
                draw.polygon([(cx + px, cy + py), (cx - px, cy - py), (tx, ty)], fill=(248, 242, 229, 250), outline=(20, 18, 16, 255), width=4)
            fill_c = (238, 231, 214, 238) if kind == "whisper" else (248, 242, 229, 250)
            draw.rounded_rectangle([bx, by, bx + bw, by + bh], radius=min(bh // 2, 42), fill=fill_c, outline=(20, 18, 16, 255), width=4 if kind == "speech" else 2)

        ty = by + pad_y
        if speaker:
            draw.text((bx + pad_x, ty), speaker, fill=(140, 74, 28, 255) if b.get("accent") == "amber" else (30, 102, 118, 255), font=f_sp)
            ty += 24
        for ln in lines:
            draw.text((bx + pad_x, ty), ln, fill=(22, 20, 18, 255), font=f_body)
            ty += 32

    out_path = out_dir / f"_preview_{page['id']}.png"
    canvas.convert("RGB").save(out_path)
    print(f"[prev] {out_path.relative_to(ROOT)}")
    return out_path


def main() -> int:
    parser = argparse.ArgumentParser(description="Gera vinhetas de mangá via text2d + previews de pranchas.")
    parser.add_argument("cutscene", help="ID da cutscene em data/cutscenes/<id>.json (ex.: prologo)")
    parser.add_argument("--only", action="append", default=[], help="Gerar apenas estes panel IDs")
    parser.add_argument("--force", action="store_true", help="Regerar mesmo que a imagem já exista")
    parser.add_argument("--preview-only", action="store_true", help="Não correr text2d; só compor previews")
    parser.add_argument("--no-preview", action="store_true", help="Não gerar imagens _preview_*.png")
    parser.add_argument("--dry-run", action="store_true", help="Só imprimir comandos sem executar")
    parser.add_argument("--steps", type=int, default=None, help="Passos de inferência do text2d (4-16)")
    parser.add_argument("--gpu", default="auto", choices=["auto", "local", "remote"], help="Modo GPU do text2d")
    parser.add_argument("--no-vramd", action="store_true", help="Forçar text2d in-process sem vramd")
    parser.add_argument("--cpu", action="store_true", help="Forçar text2d em CPU")
    args = parser.parse_args()

    json_path = CUTSCENES_DIR / f"{args.cutscene}.json"
    if not json_path.exists():
        print(f"Erro: {json_path} não existe.", file=sys.stderr)
        return 1

    spec = json.loads(json_path.read_text(encoding="utf-8"))
    only = set(args.only)

    if not args.preview_only:
        for page in spec["pages"]:
            for panel in page["panels"]:
                if only and panel["id"] not in only:
                    continue
                if not panel.get("prompt"):
                    print(f"[skip] {panel['id']} sem prompt")
                    continue
                generate_panel(
                    panel,
                    force=args.force,
                    dry_run=args.dry_run,
                    steps=args.steps,
                    gpu=args.gpu,
                    no_vramd=args.no_vramd,
                    cpu=args.cpu,
                )

    if not args.no_preview and not args.dry_run:
        out_dir = ASSETS_PUBLIC / "cutscenes" / spec["id"]
        out_dir.mkdir(parents=True, exist_ok=True)
        for page in spec["pages"]:
            render_page_preview(spec, page, out_dir)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
