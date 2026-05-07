#!/bin/sh
set -eu
mkdir -p dist
printf '#!/bin/sh\ncat >/dev/null\necho '\''{"jsonrpc":"2.0","id":1,"result":{"contract_version":"1.0.0","output":{"decision":"built"}}}'\'''\n' > dist/generated.sh
chmod +x dist/generated.sh
