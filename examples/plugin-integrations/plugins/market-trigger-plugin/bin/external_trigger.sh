#!/bin/sh
IFS= read -r start_line
symbol="BTCUSDT"
case "$start_line" in
  *'"symbol":"ETHUSDT"'*) symbol="ETHUSDT" ;;
esac
printf '%s\n' '{"type":"ready","protocol_version":"2.0.0"}'
printf '%s\n' '{"type":"event","checkpoint":"cp-e2e","event_key":"event-external","occurred_at_ms":1711200000000,"payload":{"source":"external","symbol":"'"$symbol"'"}}'
