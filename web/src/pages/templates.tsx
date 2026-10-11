import { useState } from "react";
import { Plus } from "lucide-react";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";
import { useTemplates } from "@/lib/queries";
import type { ChangeRef } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { ErrorNotice } from "@/components/status";
import { ChangeNotice } from "@/components/change-notice";
import { TemplateLibrary } from "@/components/template-library";
import { PublishedTemplates } from "@/components/templates-list";
export function TemplatesPage({ scopes }: { scopes: string[] }) {
  const templates = useTemplates(scopes.includes("template.read"));
  const location = useLocation();
  const navigate = useNavigate();
  const [search, setSearch] = useState("");
  const [params] = useSearchParams();
  const publication = location.state as { change?: ChangeRef | null; noOp?: boolean } | null;
  const [change] = useState<ChangeRef | null>(publication?.change ?? null);
  const canPublish = scopes.includes("template.publish");
  // `key` is still honoured in the URL so a redirected detail view can mark
  // its row, but the row no longer expands — the name links to the detail page.
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Templates</h1>
          <p>Create reusable templates and manage their published revisions.</p>
        </div>
        <Button onClick={() => navigate("/templates/new")} disabled={!canPublish}>
          <Plus />
          Publish template
        </Button>
      </div>
      <ChangeNotice change={change} />
      {!change && publication?.noOp && (
        <p role="status" className="mb-5 text-sm text-muted-foreground">
          No changes were needed.
        </p>
      )}
      {!scopes.includes("template.read") ? (
        <ErrorNotice error={new Error("Template read permission is required.")} />
      ) : (
        <>
          <TemplateLibrary
            canPublish={canPublish}
            onUse={(source) => navigate("/templates/new", { state: { initialSource: source } })}
          />
          <PublishedTemplates
            templates={templates}
            search={search}
            onSearchChange={setSearch}
            selectedKey={params.get("key")}
            canPublish={canPublish}
          />
        </>
      )}
    </>
  );
}
