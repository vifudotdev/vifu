"use client";

import { Check, Link2, LoaderCircle, ShieldCheck, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { runtimeBrowserRequest } from "../lib/runtime-browser-client";
import type { AgentGatewayPairing } from "../lib/runtime-types";

export function PairGateway({ requestId }: { requestId: string | null }) {
  const [pairing, setPairing] = useState<AgentGatewayPairing | null>(null);
  const [pending, setPending] = useState<"load" | "approve" | "reject" | null>(
    requestId ? "load" : null,
  );
  const [error, setError] = useState<string | null>(
    requestId ? null : "This pairing link is incomplete.",
  );

  const load = useCallback(async () => {
    if (!requestId) return;
    setPending("load");
    setError(null);
    try {
      const result = await runtimeBrowserRequest<{ pairing: AgentGatewayPairing }>(
        `agent-gateway-pairings/${requestId}`,
      );
      setPairing(result.pairing);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Pairing request could not be loaded.");
    } finally {
      setPending(null);
    }
  }, [requestId]);

  useEffect(() => { void load(); }, [load]);

  async function resolve(action: "approve" | "reject") {
    if (!requestId) return;
    setPending(action);
    setError(null);
    try {
      const result = await runtimeBrowserRequest<{ pairing: AgentGatewayPairing }>(
        `agent-gateway-pairings/${requestId}/${action}`,
        "POST",
      );
      setPairing(result.pairing);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Pairing request could not be updated.");
    } finally {
      setPending(null);
    }
  }

  const isPending = pairing?.status === "pending";
  return (
    <main className="pair-page">
      <section className="pair-card" aria-labelledby="pair-title">
        <div className="pair-mark"><Link2 aria-hidden="true" /></div>
        <header>
          <p>Agent Gateway</p>
          <h1 id="pair-title">Authorize this device</h1>
          <span>Confirm that the Gateway you just started may connect to this Vifu installation.</span>
        </header>

        {pending === "load" ? (
          <div className="pair-state"><LoaderCircle className="spin" aria-hidden="true" /><span>Checking request</span></div>
        ) : null}
        {error ? <div className="pair-error" role="alert">{error}</div> : null}
        {pairing ? (
          <dl className="pair-details">
            <div><dt>Device</dt><dd>{machineLabel(pairing.machineId)}</dd></div>
            <div><dt>Requested</dt><dd>{formatTime(pairing.createdAt)}</dd></div>
            <div><dt>Status</dt><dd data-status={pairing.status}>{pairing.status}</dd></div>
          </dl>
        ) : null}

        {isPending ? (
          <footer>
            <button className="quiet-button" type="button" onClick={() => resolve("reject")} disabled={pending !== null}>
              <X aria-hidden="true" />Reject
            </button>
            <button className="primary-button" type="button" onClick={() => resolve("approve")} disabled={pending !== null}>
              {pending === "approve" ? <LoaderCircle className="spin" aria-hidden="true" /> : <ShieldCheck aria-hidden="true" />}
              Authorize gateway
            </button>
          </footer>
        ) : null}
        {pairing && ["approved", "consumed"].includes(pairing.status) ? (
          <div className="pair-success" role="status"><Check aria-hidden="true" /><div><strong>{pairing.status === "consumed" ? "Gateway connected" : "Gateway authorized"}</strong><span>The running Gateway {pairing.status === "consumed" ? "is connected" : "will reconnect automatically"}.</span></div></div>
        ) : null}
        {pairing && !isPending && !["approved", "consumed"].includes(pairing.status) ? (
          <div className="pair-error">This pairing request is {pairing.status}.</div>
        ) : null}
      </section>
    </main>
  );
}

function machineLabel(machineId: string): string {
  const value = machineId.replace(/^machine-/, "");
  return `Machine ${value.slice(0, 8)} ${value.slice(-6)}`;
}

function formatTime(value: string): string {
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return "Recently";
  return new Intl.DateTimeFormat("en", {
    month: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
  }).format(date);
}
