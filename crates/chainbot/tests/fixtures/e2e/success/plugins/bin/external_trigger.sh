#!/bin/sh
cat <<'JSON'
{"api_version":"1.0.0","events":[{"event_id":"event-external","workflow_id":"wf-e2e","occurred_at_ms":1711200000000,"source":"external-trigger-plugin","payload":{"source":"external","symbol":"BTCUSDT"}}]}
JSON
