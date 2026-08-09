"use client";

import { AlertTriangle, X } from "lucide-react";
import { useEffect, useId, useRef } from "react";

type RuntimeConfirmDialogProps = {
  title: string;
  description: string;
  confirmLabel: string;
  pending?: boolean;
  onCancel: () => void;
  onConfirm: () => void;
};

export function RuntimeConfirmDialog({
  title,
  description,
  confirmLabel,
  pending = false,
  onCancel,
  onConfirm,
}: RuntimeConfirmDialogProps) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const titleId = useId();

  useEffect(() => {
    dialogRef.current?.showModal();
  }, []);

  return (
    <dialog
      aria-labelledby={titleId}
      className="resource-dialog runtime-confirm-dialog"
      ref={dialogRef}
      onCancel={(event) => {
        if (pending) event.preventDefault();
      }}
      onClose={onCancel}
      onClick={(event) => {
        if (!pending && event.target === event.currentTarget) event.currentTarget.close();
      }}
    >
      <div className="resource-dialog-shell runtime-confirm-shell">
        <header>
          <div>
            <span>Confirm action</span>
            <h2 id={titleId}>{title}</h2>
          </div>
          <button
            aria-label="Close"
            className="icon-button"
            disabled={pending}
            type="button"
            onClick={() => dialogRef.current?.close()}
          >
            <X aria-hidden="true" />
          </button>
        </header>
        <div className="runtime-confirm-body">
          <span className="runtime-confirm-icon"><AlertTriangle aria-hidden="true" /></span>
          <p>{description}</p>
        </div>
        <footer>
          <span />
          <span>
            <button className="secondary-button" disabled={pending} type="button" onClick={() => dialogRef.current?.close()}>Cancel</button>
            <button className="primary-button destructive-button" disabled={pending} type="button" onClick={onConfirm}>
              {pending ? "Removing" : confirmLabel}
            </button>
          </span>
        </footer>
      </div>
    </dialog>
  );
}
