"use client";

import Link from "next/link";
import { ChevronDown, FolderKanban, Plus, Search } from "lucide-react";
import { useState } from "react";
import type { RuntimeProject } from "../lib/runtime-types";
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
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const visibleApps = normalizedQuery
    ? projects.filter((project) => project.name.toLocaleLowerCase().includes(normalizedQuery))
    : projects;

  return (
    <nav className="project-breadcrumb" aria-label="Project">
      <DismissibleDetails className="project-switcher">
        <summary>
          <span className="project-avatar"><FolderKanban aria-hidden="true" /></span>
          <strong>{selectedProject?.name ?? "Create project"}</strong>
          <ChevronDown aria-hidden="true" />
        </summary>
        <div className="project-menu">
          <label className="project-search">
            <Search aria-hidden="true" />
            <input
              type="search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search projects..."
              aria-label="Search projects"
            />
          </label>
          <span>projects</span>
          <div className="project-menu-list">
            {visibleApps.length > 0 ? visibleApps.map((project) => (
              <Link key={project.id} href={`/project/${project.slug}/${activeSection}`} prefetch={false} title={project.name}>
                <strong>{project.name}</strong>
              </Link>
            )) : <p className="project-search-empty">No matching projects</p>}
          </div>
          <section className="project-create-panel">
            <div className="project-create-header"><Plus aria-hidden="true" /><span>Create project</span></div>
            <ProjectCreateForm variant="menu" />
          </section>
        </div>
      </DismissibleDetails>
    </nav>
  );
}
