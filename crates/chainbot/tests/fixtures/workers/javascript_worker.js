const fs = require("fs");

function main() {
  const raw = fs.readFileSync(0, "utf8");
  const request = JSON.parse(raw);
  const payload = request.payload ?? {};

  const response = {
    protocol_version: request.protocol_version,
    request_id: request.request_id,
    success: true,
    output: {
      runtime: "javascript",
      worker_id: request.worker_id,
      workflow_id: request.workflow_id,
      payload,
    },
  };

  process.stdout.write(JSON.stringify(response));
}

main();
