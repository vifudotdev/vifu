#!/usr/bin/env node

const readline = require("node:readline");

const lines = readline.createInterface({ input: process.stdin });
let started = false;
let sawDecode = false;

function send(message) {
  process.stdout.write(`${JSON.stringify(message)}\n`);
}

lines.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.type === "start") {
    started = true;
    send({ type: "started" });
    send({
      type: "invoke",
      id: "fixture-invocation",
      request: {
        endpoint: "guide",
        sessionId: "gateway-session",
        input: { prompt: "from Gateway" },
        metadata: { source: "test-host" },
      },
    });
    return;
  }
  if (message.type === "result" && message.id === "fixture-invocation") {
    const text = message.output?.text;
    if (text !== "Local answer: from Gateway") {
      process.stderr.write("Gateway host received an incorrect provider result\n");
      process.exit(2);
    }
    if (!sawDecode) {
      process.stderr.write("Gateway host did not receive the provider stage\n");
      process.exit(3);
    }
    send({ type: "status", state: "connected", gatewayId: "gateway-fixture" });
    return;
  }
  if (
    message.type === "trace" &&
    message.id === "fixture-invocation" &&
    message.event?.type === "stageCompleted" &&
    message.event?.stage === "decode"
  ) {
    sawDecode = true;
    return;
  }
  if (message.type === "stop" && started) {
    process.exit(0);
  }
});
