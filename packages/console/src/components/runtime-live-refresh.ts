"use client";

import { useEffect } from "react";
import { useRuntimeConsoleRouter } from "../host";

export const DEVICE_STATUS_REFRESH_MS = 5_000;

export function useRuntimeLiveRefresh(
  enabled = true,
  intervalMs = DEVICE_STATUS_REFRESH_MS,
) {
  const router = useRuntimeConsoleRouter();

  useEffect(() => {
    if (!enabled) return;

    const refreshWhenVisible = () => {
      if (document.visibilityState === "visible") router.refresh();
    };
    const onVisibilityChange = () => {
      if (document.visibilityState === "visible") router.refresh();
    };
    const timer = window.setInterval(refreshWhenVisible, intervalMs);

    window.addEventListener("focus", refreshWhenVisible);
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("focus", refreshWhenVisible);
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, [enabled, intervalMs, router]);
}
