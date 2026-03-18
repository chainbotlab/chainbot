import json
import sys


def main() -> int:
    raw = sys.stdin.read()
    request = json.loads(raw)
    payload = request.get("payload", {})

    response = {
        "protocol_version": request.get("protocol_version", "1.0.0"),
        "request_id": request.get("request_id"),
        "success": True,
        "output": {
            "script_status": "ok",
            "symbol": payload.get("symbol"),
        },
    }
    sys.stdout.write(json.dumps(response))
    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
