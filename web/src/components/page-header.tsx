import type { ReactNode } from "react";
import { ArrowLeft } from "lucide-react";
import { Link } from "react-router-dom";

/**
 * The page title block: an optional link to the parent page, the title with
 * its facts beneath, and the page's actions aligned to the right.
 */
export function PageHeader({
  back,
  title,
  meta,
  actions,
}: {
  /** `label` is the visible parent name; the link is announced as "Back to …". */
  back?: { to: string; label: string };
  title: ReactNode;
  meta?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <div data-slot="page-header" className="mb-6 min-w-0">
      {back && (
        <Link className="back-link" to={back.to} aria-label={`Back to ${back.label.toLowerCase()}`}>
          <ArrowLeft className="size-4" aria-hidden="true" />
          {back.label}
        </Link>
      )}
      <div className="flex flex-wrap items-start justify-between gap-x-6 gap-y-3">
        <div className="min-w-0 flex-1 basis-64">
          <h1 className="break-words">{title}</h1>
          {meta && (
            <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1.5 text-sm text-muted-foreground [overflow-wrap:anywhere]">
              {meta}
            </div>
          )}
        </div>
        {actions && <div className="flex flex-wrap items-center gap-2">{actions}</div>}
      </div>
    </div>
  );
}
