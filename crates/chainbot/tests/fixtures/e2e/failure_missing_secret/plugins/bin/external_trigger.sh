#!/bin/sh
input="$(cat)"
symbol="BTCUSDT"
case "$input" in
  *'"symbol":"ETHUSDT"'*) symbol="ETHUSDT" ;;
esac
cat <<JSON
{"api_version":"2.0.0","events":[{"event_id":"event-external","occurred_at_ms":1711200000000,"source":"external-trigger-plugin","payload":{"source":"external","symbol":"$symbol"}}]}
JSON
