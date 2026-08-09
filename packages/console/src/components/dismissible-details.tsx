"use client";

import type { ReactNode } from "react";
import { useEffect, useRef } from "react";

export function DismissibleDetails({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  const detailsRef = useRef<HTMLDetailsElement>(null);

  useEffect(() => {
    function closeDetails() {
      const details = detailsRef.current;
      if (details?.open) details.open = false;
    }

    function handleDocumentClick(event: MouseEvent) {
      const details = detailsRef.current;
      if (!details?.open) return;
      if (event.target instanceof Node && details.contains(event.target)) return;
      closeDetails();
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") closeDetails();
    }

    document.addEventListener("click", handleDocumentClick);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("click", handleDocumentClick);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, []);

  return (
    <details ref={detailsRef} className={className}>
      {children}
    </details>
  );
}
