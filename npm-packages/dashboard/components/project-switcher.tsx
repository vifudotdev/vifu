"use client";

import Link from "next/link";
import { ChevronDown, FolderKanban, Plus, Search, Upload } from "lucide-react";
import { useRouter } from "next/navigation";
import { useRef, useState } from "react";
import { importProjectArchive, readProjectArchive } from "../lib/project-archive";
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
  const [importing, setImporting] = useState(false);
  const [importMessage, setImportMessage] = useState<string | null>(null);
  const [importError, setImportError] = useState(false);
  const importInput = useRef<HTMLInputElement>(null);
  const router = useRouter();
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const visibleProjects = normalizedQuery
    ? projects.filter((project) => project.name.toLocaleLowerCase().includes(normalizedQuery))
    : projects;

  async function importProject(file: File | undefined) {
    if (!file) return;
    setImporting(true);
    setImportError(false);
    setImportMessage("Checking project file...");
    try {
      const archive = await readProjectArchive(file);
      const project = await importProjectArchive(archive, setImportMessage);
      router.push(`/project/${project.slug}/${activeSection}`);
      router.refresh();
    } catch (error) {
      setImportError(true);
      setImportMessage(error instanceof Error ? error.message : "The project could not be imported.");
    } finally {
      setImporting(false);
      if (importInput.current) importInput.current.value = "";
    }
  }

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
          <span>Projects</span>
          <div className="project-menu-list">
            {visibleProjects.length > 0 ? visibleProjects.map((project) => (
              <Link key={project.id} href={`/project/${project.slug}/${activeSection}`} prefetch={false} title={project.name}>
                <strong>{project.name}</strong>
              </Link>
            )) : <p className="project-search-empty">No matching projects</p>}
          </div>
          <section className="project-create-panel">
            <div className="project-create-header"><Plus aria-hidden="true" /><span>Create project</span></div>
            <ProjectCreateForm variant="menu" />
            <div className="project-import-divider"><span>or</span></div>
            <input
              ref={importInput}
              className="sr-only"
              type="file"
              accept=".vf,application/vnd.vifu.project+json"
              onChange={(event) => void importProject(event.currentTarget.files?.[0])}
            />
            <button className="project-import-button" type="button" disabled={importing} onClick={() => importInput.current?.click()}>
              <Upload aria-hidden="true" />{importing ? "Importing..." : "Import .vf project"}
            </button>
            {importMessage ? <p className={`project-import-message${importError ? " error" : ""}`} role={importError ? "alert" : "status"}>{importMessage}</p> : null}
          </section>
        </div>
      </DismissibleDetails>
    </nav>
  );
}
