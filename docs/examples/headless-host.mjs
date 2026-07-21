import { readFile } from "node:fs/promises";

const gameUrl = requiredEnvironment("VIFU_GAME_URL").replace(/\/+$/, "");
const apiKey = requiredEnvironment("VIFU_API_KEY");
const capabilities = commaSeparated(process.env.VIFU_HOST_CAPABILITIES);
const bindings = await loadBindings(process.env.VIFU_HOST_BINDINGS);
const events = [];
let revision = 0;
let lastEventId = 0;

const release = await request("");
const manifestResponse = await request("/manifest");
const manifest = manifestResponse.manifest;
validateHost(manifest, capabilities, bindings);

const created = await request("/sessions", {
  method: "POST",
  body: {
    host: {
      engine: "headless-node",
      adapterVersion: "vifu-example-v1",
      capabilities,
      locale: process.env.VIFU_HOST_LOCALE || "en",
    },
  },
});

const sessionId = created.session.id;
revision = created.session.revision;
console.log(`Session ${sessionId} uses release ${release.game.releaseNumber}.`);

let advance = await command("game.start", {});
for (let step = 0; step < 100; step += 1) {
  rememberEvents(advance.events || []);
  if (advance.status === "completed") {
    const replay = await replayEvents(0, 1_000);
    const ending = [...replay].reverse().find((event) => event.type === "ending.reached");
    console.log(`Completed${ending ? ` at ${ending.subject || "an ending"}` : ""}.`);
    console.log(JSON.stringify(advance.publicOutput ?? null, null, 2));
    process.exit(0);
  }
  if (advance.status === "failed" || advance.status === "cancelled") {
    throw new Error(`Session ${advance.status}: ${advance.failure?.message || "runtime stopped"}`);
  }
  if (advance.status === "waiting_host") {
    const action = advance.outstandingHostActions?.[0];
    if (!action) throw new Error("The runtime is waiting for a host action but returned none.");
    const binding = bindings[action.target];
    if (!binding) throw new Error(`No host binding exists for ${action.target}.`);
    console.log(`Host action ${action.action} -> ${JSON.stringify(binding.value)}`);
    advance = await command("host.action.completed", { actionId: action.actionId });
    continue;
  }
  if (advance.status === "waiting_input") {
    const choice = latestUnansweredChoice();
    if (choice) {
      const option = Array.isArray(choice.data?.options) ? choice.data.options[0] : null;
      if (!option?.id) throw new Error("A choice was presented without a selectable option.");
      choice.answered = true;
      console.log(`Choice ${choice.subject || "choice"} -> ${option.id}`);
      advance = await command("player.choice", { optionId: option.id });
    } else {
      advance = await command("player.text", {
        text: process.env.VIFU_PLAYER_TEXT || "Continue",
      });
    }
    continue;
  }
  if (advance.status === "waiting_effect") {
    advance = await waitForEffect(sessionId);
    continue;
  }
  throw new Error(`Unsupported runtime status: ${advance.status}`);
}

throw new Error("The fixture exceeded its 100-step safety limit.");

async function command(type, data) {
  const response = await request(`/sessions/${sessionId}/commands`, {
    method: "POST",
    body: {
      idempotencyKey: `headless:${crypto.randomUUID()}`,
      expectedRevision: revision,
      type,
      data,
    },
  });
  revision = response.advance.revision;
  return response.advance;
}

async function waitForEffect(sessionId) {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    await sleep(250);
    const replayed = await replayEvents(lastEventId, 300);
    rememberEvents(replayed);
    const response = await request(`/sessions/${sessionId}`);
    revision = response.session.revision;
    if (response.session.status !== "waiting_effect") {
      return {
        status: response.session.status,
        revision: response.session.revision,
        publicOutput: response.session.publicOutput,
        outstandingHostActions: response.session.outstandingHostActions,
        failure: response.session.failure,
        events: replayed,
      };
    }
  }
  throw new Error("Timed out waiting for the Agent effect worker.");
}

async function replayEvents(afterSequence, timeoutMs) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  const replayed = [];
  try {
    const response = await fetch(`${gameUrl}/sessions/${sessionId}/events`, {
      headers: {
        Accept: "text/event-stream",
        Authorization: `Bearer ${apiKey}`,
        "Last-Event-ID": String(afterSequence),
      },
      signal: controller.signal,
    });
    if (!response.ok || !response.body) throw new Error(`Event stream returned ${response.status}.`);
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    while (true) {
      const { done, value } = await reader.read();
      buffer += decoder.decode(value || new Uint8Array(), { stream: !done });
      const records = buffer.split("\n\n");
      buffer = records.pop() || "";
      for (const record of records) {
        const data = record.split("\n").filter((line) => line.startsWith("data:"));
        if (data.length === 0) continue;
        replayed.push(JSON.parse(data.map((line) => line.slice(5).trimStart()).join("\n")));
      }
      if (done) break;
    }
  } catch (error) {
    if (error?.name !== "AbortError") throw error;
  } finally {
    clearTimeout(timer);
  }
  return replayed;
}

async function request(path, options = {}) {
  const response = await fetch(`${gameUrl}${path}`, {
    method: options.method || "GET",
    headers: {
      Accept: "application/json",
      Authorization: `Bearer ${apiKey}`,
      ...(options.body ? { "Content-Type": "application/json" } : {}),
    },
    body: options.body ? JSON.stringify(options.body) : undefined,
  });
  const payload = await response.json().catch(() => null);
  if (!response.ok) {
    throw new Error(payload?.error?.message || `Vifu returned HTTP ${response.status}.`);
  }
  return payload;
}

function validateHost(manifest, supported, hostBindings) {
  const missingCapabilities = (manifest.requiredHostCapabilities || []).filter(
    (capability) => !supported.includes(capability),
  );
  if (missingCapabilities.length > 0) {
    throw new Error(`Missing host capabilities: ${missingCapabilities.join(", ")}`);
  }
  const missingBindings = (manifest.logicalResources || [])
    .filter((resource) => resource.required && !hostBindings[resource.id])
    .map((resource) => resource.id);
  if (missingBindings.length > 0) {
    throw new Error(`Missing required host bindings: ${missingBindings.join(", ")}`);
  }
}

function rememberEvents(nextEvents) {
  for (const event of nextEvents) {
    if (events.some((stored) => stored.id === event.id)) continue;
    events.push({ ...event, answered: false });
    lastEventId = Math.max(lastEventId, Number(event.sequence) || 0);
    console.log(`[${event.sequence}] ${event.type}${event.subject ? ` (${event.subject})` : ""}`);
  }
}

function latestUnansweredChoice() {
  return [...events].reverse().find((event) => event.type === "choice.presented" && !event.answered);
}

async function loadBindings(path) {
  if (!path) return {};
  const document = JSON.parse(await readFile(path, "utf8"));
  return document.bindings || {};
}

function commaSeparated(value) {
  return (value || "").split(",").map((item) => item.trim()).filter(Boolean);
}

function requiredEnvironment(name) {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`${name} is required.`);
  return value;
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
