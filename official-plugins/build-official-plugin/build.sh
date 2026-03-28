#!/bin/sh
set -eu
mkdir -p dist
printf '#!/bin/sh\necho built\n' > dist/generated.sh
chmod +x dist/generated.sh
