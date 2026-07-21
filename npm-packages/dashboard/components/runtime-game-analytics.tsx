import { Activity, CheckCircle2, Clock3, Flag, MousePointerClick, TriangleAlert } from "lucide-react";
import type { GameAnalytics } from "../lib/runtime-types";

export function RuntimeGameAnalytics({ analytics }: { analytics?: GameAnalytics }) {
  const total = analytics?.totalSessions ?? 0;
  const completed = statusCount(analytics, "completed");
  const failed = statusCount(analytics, "failed");
  const choices = eventCount(analytics, "choice.selected");
  const endings = eventCount(analytics, "ending.reached");
  const completionRate = total === 0 ? 0 : Math.round((completed / total) * 100);

  return (
    <div className="game-analytics-page">
      <section className="game-analytics-metrics">
        <AnalyticsMetric icon={Activity} label="Sessions" value={String(total)} detail={total > 0 ? `${formatDuration(analytics?.averageDurationMs ?? 0)} average runtime` : "All durable game runs"} />
        <AnalyticsMetric icon={CheckCircle2} label="Completion" value={`${completionRate}%`} detail={`${completed} completed`} />
        <AnalyticsMetric icon={MousePointerClick} label="Choices" value={String(choices)} detail="Player decisions" />
        <AnalyticsMetric icon={Flag} label="Endings" value={String(endings)} detail="Reached endings" />
      </section>

      <div className="game-analytics-grid">
        <section className="analytics-panel">
          <header><div><strong>Runtime events</strong><span>Sanitized facts from committed sessions</span></div></header>
          {analytics && analytics.events.length > 0 ? (
            <div className="analytics-event-list">
              {analytics.events.map((event) => {
                const width = Math.max(4, Math.round((event.count / Math.max(...analytics.events.map((item) => item.count))) * 100));
                return <article key={event.eventType}><code>{event.eventType}</code><span><i style={{ width: `${width}%` }} /></span><strong>{event.count}</strong></article>;
              })}
            </div>
          ) : <AnalyticsEmpty />}
        </section>

        <section className="analytics-panel session-breakdown">
          <header><div><strong>Session outcomes</strong><span>Current status distribution</span></div></header>
          {analytics && analytics.sessionStatuses.length > 0 ? (
            <div className="analytics-status-list">
              {analytics.sessionStatuses.map((status) => <article key={status.status}><span className={`session-state ${status.status}`}><i />{label(status.status)}</span><strong>{status.count}</strong></article>)}
            </div>
          ) : <AnalyticsEmpty />}
          {failed > 0 ? <p className="analytics-failure-note"><TriangleAlert aria-hidden="true" />{failed} session{failed === 1 ? "" : "s"} need investigation in Logs.</p> : null}
        </section>
      </div>

      <section className="analytics-panel analytics-session-table">
        <header><div><strong>Recent sessions</strong><span>Latest runtime activity</span></div></header>
        {analytics && analytics.recentSessions.length > 0 ? (
          <div className="analytics-session-rows">
            {analytics.recentSessions.map((session) => (
              <article key={session.id}>
                <code>{session.id.slice(0, 12)}</code>
                <span className={`session-state ${session.status}`}><i />{label(session.status)}</span>
                <span>Revision {session.revision}</span>
                <time><Clock3 aria-hidden="true" />{formatTime(session.createdAt)}</time>
              </article>
            ))}
          </div>
        ) : <AnalyticsEmpty />}
      </section>
    </div>
  );
}

function AnalyticsMetric({ icon: Icon, label: metricLabel, value, detail }: { icon: typeof Activity; label: string; value: string; detail: string }) {
  return <article><span><Icon aria-hidden="true" /></span><div><small>{metricLabel}</small><strong>{value}</strong><p>{detail}</p></div></article>;
}

function AnalyticsEmpty() {
  return <div className="analytics-empty"><Activity aria-hidden="true" /><span>Run a published release to collect runtime facts.</span></div>;
}

function statusCount(analytics: GameAnalytics | undefined, status: string): number {
  return analytics?.sessionStatuses.find((item) => item.status === status)?.count ?? 0;
}

function eventCount(analytics: GameAnalytics | undefined, type: string): number {
  return analytics?.events.find((item) => item.eventType === type)?.count ?? 0;
}

function label(value: string): string {
  return value.replaceAll("_", " ").replace(/^./, (letter) => letter.toUpperCase());
}

function formatTime(value: string): string {
  return new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(new Date(value));
}

function formatDuration(value: number): string {
  if (value < 1_000) return `${Math.max(0, Math.round(value))} ms`;
  if (value < 60_000) return `${(value / 1_000).toFixed(1)} s`;
  return `${(value / 60_000).toFixed(1)} min`;
}
