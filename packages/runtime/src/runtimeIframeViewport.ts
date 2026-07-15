type RuntimeIframeViewportSyncState = {
  animationFrame: number;
  lastPosted: RuntimeIframeViewport | null;
  activeLayoutTransitions: number;
};

type RuntimeIframeViewport = {
  width: number;
  height: number;
  devicePixelRatio: number;
};

const viewportSyncStates = new WeakMap<HTMLIFrameElement, RuntimeIframeViewportSyncState>();

function viewportSyncState(iframe: HTMLIFrameElement): RuntimeIframeViewportSyncState {
  let state = viewportSyncStates.get(iframe);
  if (!state) {
    state = { animationFrame: 0, lastPosted: null, activeLayoutTransitions: 0 };
    viewportSyncStates.set(iframe, state);
  }
  return state;
}

function clearRuntimeIframeViewportSync(iframe: HTMLIFrameElement) {
  if (typeof window === "undefined") return;
  const state = viewportSyncStates.get(iframe);
  if (!state) return;
  if (state.animationFrame) {
    window.cancelAnimationFrame(state.animationFrame);
    state.animationFrame = 0;
  }
  state.activeLayoutTransitions = 0;
}

function runtimeIframeViewport(iframe: HTMLIFrameElement): RuntimeIframeViewport | null {
  if (!iframe || typeof window === "undefined") return null;
  if (!iframe.isConnected) return null;
  const width = iframe.clientWidth;
  const height = iframe.clientHeight;
  if (width <= 0 || height <= 0) return null;
  return {
    width,
    height,
    devicePixelRatio: window.devicePixelRatio || 1,
  };
}

function sameRuntimeIframeViewport(a: RuntimeIframeViewport | null, b: RuntimeIframeViewport): boolean {
  return Boolean(a && a.width === b.width && a.height === b.height && a.devicePixelRatio === b.devicePixelRatio);
}

export function postRuntimeIframeViewport(iframe: HTMLIFrameElement | null, options: { force?: boolean } = {}) {
  if (!iframe || typeof window === "undefined") return;
  const viewport = runtimeIframeViewport(iframe);
  if (!viewport) return;
  const state = viewportSyncState(iframe);
  if (!options.force && sameRuntimeIframeViewport(state.lastPosted, viewport)) return;
  state.lastPosted = viewport;
  iframe.contentWindow?.postMessage({
    source: "vifu-web-host",
    message: {
      jsonrpc: "2.0",
      method: "host.resize",
      params: viewport,
    },
  }, "*");
}

function scheduleRuntimeIframeViewport(iframe: HTMLIFrameElement | null, options: { force?: boolean } = {}) {
  if (!iframe || typeof window === "undefined") return;
  const state = viewportSyncState(iframe);
  if (state.animationFrame) window.cancelAnimationFrame(state.animationFrame);
  state.animationFrame = window.requestAnimationFrame(() => {
    state.animationFrame = 0;
    postRuntimeIframeViewport(iframe, options);
  });
}

export function syncRuntimeIframeViewport(iframe: HTMLIFrameElement | null, options: { force?: boolean } = {}) {
  scheduleRuntimeIframeViewport(iframe, options);
}

export function observeRuntimeIframeViewport(iframe: HTMLIFrameElement | null) {
  if (!iframe || typeof window === "undefined") return undefined;
  const observedElements = new Set<Element>([iframe]);
  if (iframe.parentElement) observedElements.add(iframe.parentElement);
  const layoutRoot = iframe.closest(".app-shell-main, .app-shell-route-stage");
  const appShellMain = iframe.closest(".app-shell-main") ?? document.querySelector(".app-shell-main");
  const routeStage = iframe.closest(".app-shell-route-stage");
  const appSidebar = document.querySelector(".app-sidebar");
  for (const element of [layoutRoot, appShellMain, routeStage, appSidebar]) {
    if (element) observedElements.add(element);
  }
  const state = viewportSyncState(iframe);
  const syncLayoutEnd = () => {
    syncRuntimeIframeViewport(iframe);
  };
  const syncObservedResize = () => {
    if (state.activeLayoutTransitions > 0) return;
    syncRuntimeIframeViewport(iframe);
  };
  const isRelevantTransition = (event: Event) => {
    if (event.target !== event.currentTarget) return;
    const propertyName = "propertyName" in event ? String((event as TransitionEvent).propertyName) : "";
    return !propertyName || ["padding-left", "width", "transform"].includes(propertyName);
  };
  const handleTransitionStart = (event: Event) => {
    if (!isRelevantTransition(event)) return;
    state.activeLayoutTransitions += 1;
  };
  const handleTransitionEnd = (event: Event) => {
    if (!isRelevantTransition(event)) return;
    state.activeLayoutTransitions = Math.max(0, state.activeLayoutTransitions - 1);
    if (state.activeLayoutTransitions === 0) syncLayoutEnd();
  };
  const resizeObserver = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(syncObservedResize);
  if (resizeObserver) {
    for (const element of observedElements) {
      resizeObserver.observe(element);
    }
  }
  for (const eventName of ["transitionstart"] as const) {
    for (const element of observedElements) {
      element.addEventListener(eventName, handleTransitionStart);
    }
  }
  for (const eventName of ["transitionend", "transitioncancel"] as const) {
    for (const element of observedElements) {
      element.addEventListener(eventName, handleTransitionEnd);
    }
  }
  window.addEventListener("resize", syncLayoutEnd);
  window.addEventListener("orientationchange", syncLayoutEnd);
  syncRuntimeIframeViewport(iframe, { force: true });
  return () => {
    clearRuntimeIframeViewportSync(iframe);
    for (const eventName of ["transitionstart"] as const) {
      for (const element of observedElements) {
        element.removeEventListener(eventName, handleTransitionStart);
      }
    }
    for (const eventName of ["transitionend", "transitioncancel"] as const) {
      for (const element of observedElements) {
        element.removeEventListener(eventName, handleTransitionEnd);
      }
    }
    window.removeEventListener("resize", syncLayoutEnd);
    window.removeEventListener("orientationchange", syncLayoutEnd);
    resizeObserver?.disconnect();
  };
}

export function runtimeReadyMessageMethod(event: MessageEvent, iframe: HTMLIFrameElement | null): string | null {
  if (!iframe?.contentWindow || event.source !== iframe.contentWindow) return null;
  const data = event.data;
  if (!data || typeof data !== "object" || data.source !== "vifu-godot-runtime") return null;
  const rawMessage = "message" in data ? data.message : null;
  const message = typeof rawMessage === "string"
    ? (() => {
      try {
        return JSON.parse(rawMessage);
      } catch {
        return null;
      }
    })()
    : rawMessage;
  return message && typeof message === "object" && typeof message.method === "string" ? message.method : null;
}
