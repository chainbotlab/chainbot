import json
import os
import sys
import time


def main() -> int:
    raw = sys.stdin.read()
    request = json.loads(raw)
    payload = request.get("payload", {})
    mode = payload.get("mode", "echo")

    if mode == "sleep":
        duration_ms = int(payload.get("duration_ms", 300))
        time.sleep(duration_ms / 1000.0)
    elif mode == "sleep_with_pid":
        pid_file = payload["pid_file"]
        with open(pid_file, "w", encoding="utf-8") as handle:
            handle.write(str(os.getpid()))
        duration_ms = int(payload.get("duration_ms", 300))
        time.sleep(duration_ms / 1000.0)
    elif mode == "oversized_stdout":
        size = int(payload.get("size", 4096))
        sys.stdout.write("x" * size)
        sys.stdout.flush()
        return 0
    elif mode == "oversized_stderr":
        size = int(payload.get("size", 4096))
        sys.stderr.write("e" * size)
        sys.stderr.flush()
    elif mode == "malformed":
        sys.stdout.write("not-json-response")
        sys.stdout.flush()
        return 0
    elif mode == "future_protocol":
        response = {
            "protocol_version": "9.0.0",
            "request_id": request.get("request_id"),
            "success": True,
            "output": {"runtime": "python"},
        }
        sys.stdout.write(json.dumps(response))
        sys.stdout.flush()
        return 0

    response = {
        "protocol_version": request.get("protocol_version"),
        "request_id": request.get("request_id"),
        "success": True,
        "output": {
            "runtime": "python",
            "worker_id": request.get("worker_id"),
            "workflow_id": request.get("workflow_id"),
            "payload": payload,
        },
    }
    sys.stdout.write(json.dumps(response))
    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
