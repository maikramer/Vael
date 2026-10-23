#!/usr/bin/env bash
# Corre um modelo no llama-server, avalia e mata o servidor.
# uso: ./run_variant.sh <modelo.gguf> <nome> <porta> <bench.jsonl> [extra]
set -u
LAB="$(cd "$(dirname "$0")" && pwd)"
MODEL="$1"; NAME="$2"; PORT="$3"; BENCH="${4:-bench2.jsonl}"; EXTRA="${5:-}"
BIN="$LAB/llama/llama-b11669/llama-server"
LOG="$LAB/logs/$NAME.log"
mkdir -p "$LAB/logs"
pkill -f "llama-server.*--port $PORT" 2>/dev/null || true
sleep 0.5
"$BIN" -m "$MODEL" --host 127.0.0.1 --port "$PORT" -c 4096 -t 8 --no-webui $EXTRA > "$LOG" 2>&1 &
PID=$!
for i in $(seq 1 120); do
  if ! kill -0 "$PID" 2>/dev/null; then echo "servidor morreu; log:"; tail -20 "$LOG"; exit 1; fi
  if curl -sS --max-time 2 "http://127.0.0.1:$PORT/health" 2>/dev/null | grep -q ok; then break; fi
  sleep 0.5
done
python3 "$LAB/eval.py" --port "$PORT" --name "$NAME" --bench "$LAB/$BENCH"
RC=$?
kill "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
exit $RC
