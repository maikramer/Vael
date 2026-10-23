#!/usr/bin/env bash
# Testa /tokenize num modelo. uso: ./probe_tokenize.sh modelo.gguf porta "texto"
set -u
LAB="$(cd "$(dirname "$0")" && pwd)"
MODEL="$1"; PORT="$2"; TEXT="${3:-Dois quartos livres; um tem a janela que range.}"
BIN="$LAB/llama/llama-b11669/llama-server"
pkill -f "llama-server.*--port $PORT" 2>/dev/null || true
sleep 0.3
"$BIN" -m "$MODEL" --host 127.0.0.1 --port "$PORT" -c 4096 -t 4 --no-webui > "$LAB/logs/tok_$PORT.log" 2>&1 &
PID=$!
for i in $(seq 1 120); do
  kill -0 "$PID" 2>/dev/null || { echo "morreu"; tail -5 "$LAB/logs/tok_$PORT.log"; exit 1; }
  curl -sS --max-time 2 "http://127.0.0.1:$PORT/health" 2>/dev/null | grep -q ok && break
  sleep 0.5
done
python3 - "$PORT" "$TEXT" <<'PY'
import json,sys,urllib.request
port=sys.argv[1]; text=sys.argv[2]
body=json.dumps({"content":text}).encode()
req=urllib.request.Request(f"http://127.0.0.1:{port}/tokenize",data=body,headers={"Content-Type":"application/json"})
with urllib.request.urlopen(req,timeout=60) as r: d=json.loads(r.read())
print("n_tokens",len(d.get("tokens",[])))
print("tokens",d.get("tokens")[:60])
PY
kill "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
