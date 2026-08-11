"use client";

import { ChevronDown, FolderKanban, Plus, Search } from "lucide-react";
import { useState } from "react";
import { RuntimeLink, useRuntimeConsoleHost } from "../host";
import type { RuntimeProject } from "../types";
import type { DashboardSection } from "./runtime-console";
import { DismissibleDetails } from "./dismissible-details";
import { ProjectCreateForm } from "./runtime-actions";

export function ProjectSwitcher({
  projects,
  selectedProject,
  activeSection,
}: {
  projects: RuntimeProject[];
  selectedProject: RuntimeProject | null;
  activeSection: DashboardSection;
}) {
  const [query, setQuery] = useState("");
  const host = useRuntimeConsoleHost();
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const visibleApps = normalizedQuery
    ? projects.filter((project) => project.name.toLocaleLowerCase().includes(normalizedQuery))
    : projects;

  return (
    <nav className="project-breadcrumb" aria-label="App">
      <DismissibleDetails className="project-switcher">
        <summary>
          <span className="project-avatar"><FolderKanban aria-hidden="true" /></span>
          <strong>{selectedProject?.name ?? "Create app"}</strong>
          <ChevronDown aria-hidden="true" />
        </summary>
        <div className="project-menu">
          <label className="project-search">
            <Search aria-hidden="true" />
            <input
              type="search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search apps..."
              aria-label="Search apps"
            />
          </label>
          <span>Apps</span>
          <div className="project-menu-list">
            {visibleApps.length > 0 ? visibleApps.map((project) => (
              <RuntimeLink key={project.id} href={host.projectSectionHref(project.slug, activeSection)} prefetch={false} title={project.name}>
                <strong>{project.name}</strong>
              </RuntimeLink>
            )) : <p className="project-search-empty">No matching apps</p>}
          </div>
          <section className="project-create-panel">
            <div className="project-create-header"><Plus aria-hidden="true" /><span>Create app</span></div>
            <ProjectCreateForm variant="menu" />
          </section>
        </div>
      </DismissibleDetails>
    </nav>
  );
}
