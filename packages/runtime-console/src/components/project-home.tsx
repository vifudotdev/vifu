"use client";

import { FolderKanban, Link2, Plus, Search } from "lucide-react";
import { useMemo, useState, type FormEvent } from "react";
import { runtimeBrowserRequest } from "../browser-client";
import { RuntimeLink, useRuntimeConsoleHost, useRuntimeConsoleRouter } from "../host";
import type { RuntimeProject } from "../types";
import { DismissibleDetails } from "./dismissible-details";
import { ProjectCreateForm } from "./runtime-actions";

export function ProjectHome({
  projects,
  allowGuestClaim = false,
}: {
  projects: RuntimeProject[];
  allowGuestClaim?: boolean;
}) {
  const [query, setQuery] = useState("");
  const host = useRuntimeConsoleHost();
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const visibleProjects = useMemo(() => {
    if (!normalizedQuery) return projects;
    return projects.filter((project) => (
      project.name.toLocaleLowerCase().includes(normalizedQuery)
      || project.slug.toLocaleLowerCase().includes(normalizedQuery)
    ));
  }, [normalizedQuery, projects]);

  return (
    <div className="console-content project-home-content">
      <header className="project-home-heading">
        <div>
          <h1>Projects</h1>
          <p>Open a project or create a new agent runtime.</p>
        </div>
        <ProjectCreateMenu />
      </header>

      {projects.length > 0 ? (
        <>
          <label className="project-home-search">
            <Search aria-hidden="true" />
            <input
              type="search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search projects"
              aria-label="Search projects"
            />
          </label>
          {visibleProjects.length > 0 ? (
            <div className="project-home-grid">
              {visibleProjects.map((project) => (
                <RuntimeLink
                  className="project-home-card"
                  href={host.projectHref(project.slug)}
                  key={project.id}
                  prefetch={false}
                >
                  <span className="project-home-card-icon"><FolderKanban aria-hidden="true" /></span>
                  <span className="project-home-card-copy">
                    <strong>{project.name}</strong>
                    <small>{project.description || project.slug}</small>
                  </span>
                  <span className={`project-home-card-status${project.enabled ? "" : " disabled"}`}>
                    {project.enabled ? "Active" : "Disabled"}
                  </span>
                  <time dateTime={project.updatedAt}>Updated {formatProjectDate(project.updatedAt)}</time>
                </RuntimeLink>
              ))}
            </div>
          ) : (
            <div className="project-home-no-results">
              <Search aria-hidden="true" />
              <strong>No matching projects</strong>
              <span>Try a project name or slug.</span>
            </div>
          )}
        </>
      ) : (
        <section className="project-home-empty">
          <span className="project-home-empty-icon"><FolderKanban aria-hidden="true" /></span>
          <h2>Create your first project</h2>
          <p>A project keeps its agents, endpoints, providers, and logs together.</p>
          <ProjectCreateForm />
        </section>
      )}
      {allowGuestClaim ? <GuestProjectClaim /> : null}
    </div>
  );
}

function GuestProjectClaim() {
  const router = useRuntimeConsoleRouter();
  const [claimToken, setClaimToken] = useState("");
  const [pending, setPending] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  async function claim(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setPending(true);
    setMessage(null);
    try {
      await runtimeBrowserRequest("guest/claim", "POST", { claimToken: claimToken.trim() });
      setClaimToken("");
      router.refresh();
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "Could not claim the project.");
    } finally {
      setPending(false);
    }
  }

  return (
    <section className="guest-project-claim">
      <span className="project-home-card-icon"><Link2 aria-hidden="true" /></span>
      <div><strong>Claim a local project</strong><p>Attach a project created by the Vifu CLI to this account.</p></div>
      <form onSubmit={claim}>
        <input value={claimToken} onChange={(event) => setClaimToken(event.target.value)} required placeholder="Claim token" aria-label="Claim token" />
        <button className="secondary-button" disabled={pending} type="submit">{pending ? "Claiming" : "Claim"}</button>
      </form>
      {message ? <span className="inline-error" role="alert">{message}</span> : null}
    </section>
  );
}

function ProjectCreateMenu() {
  return (
    <DismissibleDetails className="project-home-create">
      <summary className="primary-button"><Plus aria-hidden="true" />Create project</summary>
      <div className="project-home-create-popover">
        <header>
          <strong>Create project</strong>
          <span>Name the runtime you want to operate.</span>
        </header>
        <ProjectCreateForm variant="menu" />
      </div>
    </DismissibleDetails>
  );
}

function formatProjectDate(value: string): string {
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return "recently";
  return new Intl.DateTimeFormat("en", {
    month: "short",
    day: "numeric",
    year: "numeric",
    timeZone: "UTC",
  }).format(date);
}
