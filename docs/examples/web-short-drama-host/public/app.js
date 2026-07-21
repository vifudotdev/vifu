const shell = document.querySelector(".game-shell");
const background = document.querySelector("#scene-background");
const backdrop = document.querySelector("#backdrop-image");
const characters = document.querySelector("#character-layer");
const interaction = document.querySelector("#interaction-layer");
const storyTitle = document.querySelector("#story-title");
const storyText = document.querySelector("#story-text");
const speaker = document.querySelector("#speaker");
const connectionStatus = document.querySelector("#connection-status");
const releaseLabel = document.querySelector("#release-label");
const sessionLabel = document.querySelector("#session-label");
const localeSelect = document.querySelector("#locale-select");
const soundButton = document.querySelector("#sound-button");
const restartButton = document.querySelector("#restart-button");

const state = {
  bootstrap: null,
  sessionId: null,
  revision: 0,
  status: "idle",
  events: [],
  streamAbort: null,
  commandPending: false,
  soundEnabled: true,
  audio: new Map(),
};

const copy = {
  en: {
    startEyebrow: "INTERACTIVE ANIME DRAMA",
    start: "Board the last train",
    startNote: "Your choices shape trust, identity, and the final dawn.",
    openingTitle: "The last train is waiting.",
    openingText: "A wedding. A dead lover. Thirteen minutes before midnight.",
    runtimeReady: "Runtime ready",
    ready: "Ready to begin",
    restarting: "Departing for the Moon",
    moving: "The train is moving",
    deciding: "The character is deciding",
    response: "Your response",
    responsePlaceholder: "Type your answer",
    send: "Send",
    continue: "Continue",
    hold: "Hold to break the moon's control",
    endingReached: "ENDING REACHED",
    return: "Return to 00:00",
    tryAgain: "Try again",
    connectionError: "CONNECTION ERROR",
    stopped: "RUNTIME STOPPED",
    stoppedDetail: "The session could not continue.",
    mute: "Mute sound",
    unmute: "Enable sound",
    restart: "Restart story",
    choiceLabel: "YOUR CHOICE",
    choiceDetail: "Midnight draws closer.",
    speakLabel: "SPEAK FREELY",
    speakDetail: "The carriage falls silent.",
    breakLabel: "BREAK THE COMMAND",
    breakTitle: "Your body no longer obeys you.",
    breakDetail: "Resist the moonstone control before midnight.",
    endingLabel: "ENDING",
    endingTitle: "The journey ends.",
    endingDetail: "Your choices have become the final memory.",
    mizukiTitle: "Mizuki lowers her guard.",
    mizukiDetail: "Your answer becomes part of the memory between you.",
    shionTitle: "Shion studies your answer.",
    shionDetail: "For the first time, his certainty begins to crack.",
    agentTitle: "Your words have changed the route.",
    agentDetail: "The train carries their meaning forward.",
  },
  ja: {
    startEyebrow: "インタラクティブ・アニメドラマ",
    start: "月行き最終列車に乗る",
    startNote: "あなたの選択が、信頼と正体、そして最後の夜明けを変える。",
    openingTitle: "最終列車が待っている。",
    openingText: "結婚式。死んだ恋人。午前零時まで、あと十三分。",
    runtimeReady: "接続完了",
    ready: "物語を始める",
    restarting: "月へ出発",
    moving: "列車が走り出す",
    deciding: "彼女は答えを受け止めている",
    response: "あなたの返答",
    responsePlaceholder: "答えを入力",
    send: "送る",
    continue: "続ける",
    hold: "長押しして月の支配を破る",
    endingReached: "エンディング",
    return: "00:00 に戻る",
    tryAgain: "もう一度",
    connectionError: "接続エラー",
    stopped: "物語が停止しました",
    stoppedDetail: "このセッションを続けられませんでした。",
    mute: "音を消す",
    unmute: "音を出す",
    restart: "物語を最初から始める",
    choiceLabel: "あなたの選択",
    choiceDetail: "午前零時が近づいている。",
    speakLabel: "自由に答える",
    speakDetail: "車内が静まり返る。",
    breakLabel: "命令に抗う",
    breakTitle: "身体が言うことを聞かない。",
    breakDetail: "午前零時になる前に、月石の支配を破れ。",
    endingLabel: "エンディング",
    endingTitle: "旅が終わる。",
    endingDetail: "あなたの選択が、最後の記憶になった。",
    mizukiTitle: "美月が剣を下ろす。",
    mizukiDetail: "あなたの言葉が、二人の新しい記憶になる。",
    shionTitle: "紫苑はあなたの答えを見つめる。",
    shionDetail: "初めて、彼の確信にひびが入る。",
    agentTitle: "あなたの言葉が進路を変えた。",
    agentDetail: "列車はその意味を乗せて走り続ける。",
  },
  "zh-CN": {
    startEyebrow: "互动动漫短剧",
    start: "登上月行末班列车",
    startNote: "你的选择将改变信任、身份与最后的黎明。",
    openingTitle: "末班列车正在等你。",
    openingText: "一场婚礼。死去的恋人。距离午夜只剩十三分钟。",
    runtimeReady: "运行时已连接",
    ready: "准备开始",
    restarting: "列车驶向月球",
    moving: "列车继续前进",
    deciding: "角色正在回应你的选择",
    response: "你的回答",
    responsePlaceholder: "输入回答",
    send: "发送",
    continue: "继续",
    hold: "长按挣脱月石控制",
    endingReached: "已到达结局",
    return: "返回 00:00",
    tryAgain: "重试",
    connectionError: "连接错误",
    stopped: "故事已停止",
    stoppedDetail: "当前会话无法继续。",
    mute: "静音",
    unmute: "开启声音",
    restart: "重新开始故事",
    choiceLabel: "你的选择",
    choiceDetail: "午夜正在逼近。",
    speakLabel: "自由回答",
    speakDetail: "车厢安静下来。",
    breakLabel: "挣脱命令",
    breakTitle: "你的身体不再听从自己。",
    breakDetail: "在午夜前挣脱月石的控制。",
    endingLabel: "结局",
    endingTitle: "旅程结束了。",
    endingDetail: "你的选择成为了最后的记忆。",
    mizukiTitle: "美月放下了戒备。",
    mizukiDetail: "你的回答成为了你们之间新的记忆。",
    shionTitle: "紫苑审视着你的回答。",
    shionDetail: "他的笃定第一次出现裂痕。",
    agentTitle: "你的话改变了列车的方向。",
    agentDetail: "列车载着它的意义继续前进。",
  },
};

restartButton.addEventListener("click", () => void startGame());
soundButton.addEventListener("click", () => {
  state.soundEnabled = !state.soundEnabled;
  updateSound();
});
localeSelect.addEventListener("change", () => {
  applyLocaleCopy();
  if (state.sessionId) void startGame();
  else showStart();
});

void bootstrap();

async function bootstrap() {
  try {
    state.bootstrap = await request("/api/bootstrap");
    const release = state.bootstrap.game.game;
    releaseLabel.textContent = `Release ${release.releaseNumber} · ${release.compatibility.protocol}`;
    connectionStatus.textContent = t("runtimeReady");
    const openingImage = assetUrl("rain-station");
    if (openingImage) {
      background.src = openingImage;
      backdrop.src = openingImage;
    }
    shell.dataset.state = "ready";
    applyLocaleCopy();
    showStart();
  } catch (error) {
    showError(error);
  }
}

function showStart() {
  const start = document.querySelector("#start-template").content.cloneNode(true);
  start.querySelector("span").textContent = t("startEyebrow");
  start.querySelector("button").textContent = t("start");
  start.querySelector("small").textContent = t("startNote");
  interaction.replaceChildren(start);
  interaction.querySelector("button").addEventListener("click", () => void startGame());
}

async function startGame() {
  if (!state.bootstrap || state.commandPending) return;
  state.commandPending = true;
  state.streamAbort?.abort();
  state.streamAbort = null;
  state.sessionId = null;
  state.revision = 0;
  state.status = "running";
  state.events = [];
  stopAudio();
  localeSelect.disabled = true;
  interaction.innerHTML = `<div class="runtime-wait"><strong>${escapeHtml(t("restarting"))}</strong><span>....</span></div>`;
  shell.dataset.state = "playing";
  try {
    const result = await request("/api/run", {
      method: "POST",
      body: { locale: localeSelect.value },
    });
    state.sessionId = result.sessionId;
    applyAdvance(result.advance);
    sessionLabel.textContent = `Session ${result.sessionId.slice(0, 8)}`;
    void consumeEvents(result.sessionId, highestSequence());
  } catch (error) {
    showError(error);
  } finally {
    state.commandPending = false;
    renderInteraction();
  }
}

async function sendCommand(type, data) {
  if (!state.sessionId || state.commandPending) return;
  state.commandPending = true;
  renderInteraction();
  try {
    const result = await request(`/api/sessions/${state.sessionId}/commands`, {
      method: "POST",
      body: {
        idempotencyKey: `web:${crypto.randomUUID()}`,
        expectedRevision: state.revision,
        type,
        data,
      },
    });
    applyAdvance(result.advance);
  } catch (error) {
    showError(error);
  } finally {
    state.commandPending = false;
    renderInteraction();
  }
}

async function consumeEvents(sessionId, after) {
  state.streamAbort?.abort();
  const controller = new AbortController();
  state.streamAbort = controller;
  try {
    const response = await fetch(`/api/sessions/${sessionId}/events?after=${after}`, {
      headers: { Accept: "text/event-stream" },
      signal: controller.signal,
    });
    if (!response.ok || !response.body) throw new Error(`Event stream returned HTTP ${response.status}.`);
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    while (state.sessionId === sessionId) {
      const { done, value } = await reader.read();
      buffer += decoder.decode(value || new Uint8Array(), { stream: !done });
      const records = buffer.split("\n\n");
      buffer = records.pop() || "";
      for (const record of records) {
        const data = record.split("\n").filter((line) => line.startsWith("data:"));
        if (data.length === 0) continue;
        const event = JSON.parse(data.map((line) => line.slice(5).trimStart()).join("\n"));
        mergeEvents([event]);
        void refreshSession(sessionId);
      }
      if (done) break;
    }
  } catch (error) {
    if (error.name !== "AbortError" && state.sessionId === sessionId) showError(error);
  }
}

async function refreshSession(sessionId) {
  try {
    const result = await request(`/api/sessions/${sessionId}`);
    if (state.sessionId !== sessionId) return;
    state.status = result.session.status;
    state.revision = result.session.revision;
    state.outstandingHostActions = result.session.outstandingHostActions || [];
    renderInteraction();
  } catch (error) {
    if (state.sessionId === sessionId) showError(error);
  }
}

function applyAdvance(advance) {
  state.status = advance.status;
  state.revision = advance.revision;
  state.outstandingHostActions = advance.outstandingHostActions || [];
  mergeEvents(advance.events || []);
}

function mergeEvents(incoming) {
  const stored = new Map(state.events.map((event) => [event.id, event]));
  for (const event of incoming) stored.set(event.id, event);
  state.events = [...stored.values()].sort((left, right) => left.sequence - right.sequence);
  renderPresentation();
  renderInteraction();
}

function renderPresentation() {
  const backgroundEvent = latest("background.changed");
  const backgroundUrl = assetUrl(backgroundEvent?.data?.logicalResourceId);
  if (backgroundUrl && background.src !== new URL(backgroundUrl, location.href).href) {
    background.src = backgroundUrl;
    backdrop.src = backgroundUrl;
  }

  const lastBackgroundSequence = backgroundEvent?.sequence || 0;
  const currentCharacters = new Map();
  for (const event of state.events) {
    if (event.sequence < lastBackgroundSequence || event.type !== "character.visual.changed") continue;
    const id = event.data?.characterId || event.subject;
    if (id) currentCharacters.set(id, event);
  }
  characters.replaceChildren(...[...currentCharacters.values()].slice(-3).map((event) => {
    const image = document.createElement("img");
    image.src = assetUrl(event.data?.logicalResourceId) || "";
    image.alt = "";
    return image;
  }).filter((image) => image.src));

  syncAudio();
  const visible = latestVisibleEvent();
  const copy = eventCopy(visible);
  speaker.textContent = copy.label;
  storyTitle.textContent = copy.title;
  storyText.textContent = copy.detail;
  storyTitle.classList.toggle("long", copy.title.length > 74);
  storyTitle.classList.toggle("very-long", copy.title.length > 130);
  connectionStatus.textContent = statusLabel(state.status);
}

function renderInteraction() {
  if (!state.sessionId) return;
  interaction.replaceChildren();
  if (state.commandPending || state.status === "waiting_effect") {
    interaction.innerHTML = `<div class="runtime-wait"><strong>${escapeHtml(state.status === "waiting_effect" ? t("deciding") : t("moving"))}</strong><span>....</span></div>`;
    return;
  }

  if (state.status === "completed") {
    const ending = latest("ending.reached");
    interaction.innerHTML = `<div class="ending-panel"><span>${escapeHtml(t("endingReached"))}</span><small>${escapeHtml(ending?.data?.title || eventCopy(ending).title)}</small><button type="button">${escapeHtml(t("return"))}</button></div>`;
    interaction.querySelector("button").addEventListener("click", () => void startGame());
    localeSelect.disabled = false;
    return;
  }

  if (["failed", "cancelled"].includes(state.status)) {
    interaction.innerHTML = `<div class="ending-panel"><span>${escapeHtml(t("stopped"))}</span><small>${escapeHtml(t("stoppedDetail"))}</small><button type="button">${escapeHtml(t("tryAgain"))}</button></div>`;
    interaction.querySelector("button").addEventListener("click", () => void startGame());
    localeSelect.disabled = false;
    return;
  }

  if (state.status === "waiting_host") {
    renderHostAction();
    return;
  }

  if (state.status !== "waiting_input") return;
  const waiting = latest("game.session.waiting");
  const commandType = waiting?.data?.commandType;
  if (commandType === "player.choice") {
    renderChoices();
  } else if (commandType === "player.continue") {
    const button = document.createElement("button");
    button.className = "continue-button";
    button.type = "button";
    button.textContent = t("continue");
    button.addEventListener("click", () => void sendCommand("player.continue", {}));
    interaction.append(button);
  } else if (commandType) {
    renderTextInput(commandType);
  }
}

function renderChoices() {
  const choice = latest("choice.presented");
  const list = document.createElement("div");
  list.className = "choice-list";
  for (const option of choice?.data?.options || []) {
    const button = document.createElement("button");
    button.type = "button";
    button.disabled = option.available === false;
    button.textContent = option.label || option.id;
    button.title = option.available === false ? option.lockedReason || "This path is locked" : "";
    button.addEventListener("click", () => void sendCommand("player.choice", { optionId: option.id }));
    list.append(button);
  }
  interaction.append(list);
}

function renderTextInput(commandType) {
  const form = document.createElement("form");
  form.className = "text-response";
  form.innerHTML = `<label><span>${escapeHtml(t("response"))}</span><div><input name="response" autocomplete="off" maxlength="600" placeholder="${escapeHtml(t("responsePlaceholder"))}" aria-label="${escapeHtml(t("response"))}"><button type="submit">${escapeHtml(t("send"))}</button></div></label>`;
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const text = new FormData(form).get("response")?.toString().trim();
    if (text) void sendCommand(commandType, { text });
  });
  interaction.append(form);
  form.querySelector("input").focus();
}

function renderHostAction() {
  const action = state.outstandingHostActions?.[0];
  if (!action) return;
  const wrapper = document.createElement("div");
  wrapper.className = "host-action";
  const button = document.createElement("button");
  button.type = "button";
  button.innerHTML = `<i></i><span>${escapeHtml(t("hold"))}</span>`;
  const fill = button.querySelector("i");
  let started = 0;
  let frame = 0;
  const stop = () => {
    cancelAnimationFrame(frame);
    frame = 0;
    fill.style.width = "0";
  };
  const tick = (time) => {
    const progress = Math.min(1, (time - started) / 1200);
    fill.style.width = `${progress * 100}%`;
    if (progress >= 1) {
      void sendCommand("host.action.completed", { actionId: action.actionId });
      frame = 0;
      return;
    }
    frame = requestAnimationFrame(tick);
  };
  button.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    started = performance.now();
    frame = requestAnimationFrame(tick);
  });
  button.addEventListener("pointerup", stop);
  button.addEventListener("pointerleave", stop);
  button.addEventListener("pointercancel", stop);
  wrapper.append(button);
  interaction.append(wrapper);
}

function syncAudio() {
  const recent = [];
  for (const event of [...state.events].reverse()) {
    if (event.type !== "audio.play") continue;
    const id = event.data?.logicalResourceId;
    if (!id || recent.includes(id)) continue;
    recent.push(id);
    if (recent.length === 2) break;
  }
  for (const [id, audio] of state.audio) {
    if (!recent.includes(id)) {
      audio.pause();
      state.audio.delete(id);
    }
  }
  for (const id of recent) {
    if (state.audio.has(id)) continue;
    const event = [...state.events].reverse().find((item) => item.type === "audio.play" && item.data?.logicalResourceId === id);
    const url = assetUrl(id);
    if (!url) continue;
    const audio = new Audio(url);
    audio.loop = event?.data?.loop === true;
    audio.volume = Math.min(1, Math.max(0, Number(event?.data?.volume ?? 1)));
    audio.muted = !state.soundEnabled;
    state.audio.set(id, audio);
    void audio.play().catch(() => {});
  }
  updateSound();
}

function updateSound() {
  for (const audio of state.audio.values()) {
    audio.muted = !state.soundEnabled;
    if (state.soundEnabled) void audio.play().catch(() => {});
  }
  soundButton.querySelector("span").textContent = state.soundEnabled ? "♪" : "×";
  soundButton.ariaLabel = state.soundEnabled ? t("mute") : t("unmute");
  soundButton.title = soundButton.ariaLabel;
}

function stopAudio() {
  for (const audio of state.audio.values()) audio.pause();
  state.audio.clear();
}

function assetUrl(logicalId) {
  const binding = state.bootstrap?.presentation?.presentation?.bindings?.[logicalId];
  return binding?.kind === "managed-asset-version" ? `/api/assets/${binding.value}` : null;
}

function latest(type) {
  return [...state.events].reverse().find((event) => event.type === type);
}

function latestVisibleEvent() {
  const visible = new Set([
    "scene.entered",
    "dialogue.started",
    "agent.completed",
    "ending.reached",
    "choice.presented",
    "player.input.requested",
    "host.action.requested",
  ]);
  return [...state.events].reverse().find((event) => visible.has(event.type));
}

function eventCopy(event) {
  if (!event) return { label: "VIFU ORIGINAL", title: t("openingTitle"), detail: t("openingText") };
  const data = event.data || {};
  if (event.type === "choice.presented") return { label: t("choiceLabel"), title: data.prompt || t("choiceLabel"), detail: t("choiceDetail") };
  if (event.type === "player.input.requested") return { label: t("speakLabel"), title: data.prompt || t("speakLabel"), detail: t("speakDetail") };
  if (event.type === "host.action.requested") return { label: t("breakLabel"), title: t("breakTitle"), detail: t("breakDetail") };
  if (event.type === "ending.reached") return { label: t("endingLabel"), title: data.title || t("endingTitle"), detail: data.description || t("endingDetail") };
  if (event.type === "agent.completed") {
    if (String(event.subject).includes("mizuki")) return { label: "MIZUKI", title: t("mizukiTitle"), detail: t("mizukiDetail") };
    if (String(event.subject).includes("shion")) return { label: "SHION", title: t("shionTitle"), detail: t("shionDetail") };
    return { label: "AGENT", title: t("agentTitle"), detail: t("agentDetail") };
  }
  return {
    label: event.type === "agent.completed" ? "AGENT RESPONSE" : "STORY",
    title: data.title || data.name || data.text || event.subject || "The train moves forward.",
    detail: data.description || data.dialogue || data.message || data.response || "Your choice has changed the route.",
  };
}

function highestSequence() {
  return state.events.reduce((highest, event) => Math.max(highest, Number(event.sequence) || 0), 0);
}

function statusLabel(value) {
  return String(value || "running").replaceAll("_", " ");
}

async function request(path, options = {}) {
  const response = await fetch(path, {
    method: options.method || "GET",
    headers: {
      Accept: "application/json",
      ...(options.body ? { "Content-Type": "application/json" } : {}),
    },
    body: options.body ? JSON.stringify(options.body) : undefined,
  });
  const payload = await response.json().catch(() => null);
  if (!response.ok) throw new Error(payload?.error?.message || `Request returned HTTP ${response.status}.`);
  return payload;
}

function showError(error) {
  console.error(error);
  connectionStatus.textContent = "Runtime error";
  shell.dataset.state = "ready";
  localeSelect.disabled = false;
  interaction.innerHTML = `<div class="ending-panel"><span>${escapeHtml(t("connectionError"))}</span><small>${escapeHtml(error instanceof Error ? error.message : t("stoppedDetail"))}</small><button type="button">${escapeHtml(t("tryAgain"))}</button></div>`;
  interaction.querySelector("button").addEventListener("click", () => state.sessionId ? void startGame() : void bootstrap());
}

function applyLocaleCopy() {
  if (!state.sessionId) {
    storyTitle.textContent = t("openingTitle");
    storyText.textContent = t("openingText");
    sessionLabel.textContent = t("ready");
  }
  restartButton.ariaLabel = t("restart");
  restartButton.title = t("restart");
  updateSound();
}

function t(key) {
  return copy[localeSelect.value]?.[key] || copy.en[key] || key;
}

function escapeHtml(value) {
  const node = document.createElement("span");
  node.textContent = String(value);
  return node.innerHTML;
}
