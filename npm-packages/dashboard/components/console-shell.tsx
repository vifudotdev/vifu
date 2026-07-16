"use client";

import { PanelLeftClose, PanelLeftOpen } from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useLayoutEffect, useState } from "react";

const SIDEBAR_STATE_KEY = "vifu.console.sidebarCollapsed";

export function AppLayout({
  children,
  header,
  sidebar,
}: {
  children: ReactNode;
  header: ReactNode;
  sidebar: ReactNode;
}) {
  const [collapsed, setCollapsed] = useState(false);
  const [motionDisabled, setMotionDisabled] = useState(false);

  useLayoutEffect(() => {
    setCollapsed(window.localStorage.getItem(SIDEBAR_STATE_KEY) === "1");
  }, []);

  useEffect(() => {
    window.localStorage.setItem(SIDEBAR_STATE_KEY, collapsed ? "1" : "0");
  }, [collapsed]);

  useEffect(() => {
    if (!motionDisabled) return;
    const timeout = window.setTimeout(() => setMotionDisabled(false), 80);
    return () => window.clearTimeout(timeout);
  }, [collapsed, motionDisabled]);

  function toggleSidebar() {
    closeOpenDetails();
    setMotionDisabled(true);
    setCollapsed((current) => !current);
  }

  const className = [
    "console-app",
    "project-console-app",
    collapsed ? "sidebar-collapsed" : "",
    motionDisabled ? "sidebar-motion-off" : "",
  ].filter(Boolean).join(" ");

  return (
    <div className={className}>
      <button
        className="console-sidebar-toggle"
        type="button"
        aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
        aria-pressed={collapsed}
        onClick={toggleSidebar}
      >
        {collapsed ? <PanelLeftOpen aria-hidden="true" /> : <PanelLeftClose aria-hidden="true" />}
        <span>{collapsed ? "Expand" : "Collapse"}</span>
      </button>
      <aside className="console-sidebar">{sidebar}</aside>
      <section className="sidebar-inset">
        <header className="app-header">{header}</header>
        <main className="console-main">{children}</main>
      </section>
    </div>
  );
}

function closeOpenDetails() {
  document.querySelectorAll("details[open]").forEach((element) => {
    if (element instanceof HTMLDetailsElement) element.open = false;
  });
}
