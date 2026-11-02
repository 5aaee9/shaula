import { PageHeader } from "@/components/page-header";
import { useState } from "react";
import { Plus } from "lucide-react";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";
import { useTemplates } from "@/lib/queries";
import type { ChangeRef } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { ErrorNotice } from "@/components/status";
import { ChangeNotice } from "@/components/change-notice";
import { QuickUpdateDialog } from "@/components/quick-update-dialog";
import { TemplateLibrary } from "@/components/template-library";
import { PublishedTemplates } from "@/components/templates-list";
export function TemplatesPage({ scopes }: { scopes: string[] }) {
  const templates = useTemplates(scopes.includes("template.read"));
  const location = useLocation();
  const navigate = useNavigate();
  const [search, setSearch] = useState("");
  const [params] = useSearchParams();
  const publication = location.state as { change?: ChangeRef | null; noOp?: boolean } | null;
  const [result, setResult] = useState(() => ({
    change: publication?.change ?? null,
    noOp: !!publication?.noOp,
  }));
  const [updating, setUpdating] = useState<string | null>(null);
  const canPublish = scopes.includes("template.publish");
  // `key` is still honoured in the URL so a redirected detail view can mark
  // its row, but the row no longer expands — the name links to the detail page.
  return (
    <>
      <PageHeader
        title="Templates"
        actions={
          <Button onClick={() => navigate("/templates/new")} disabled={!canPublish}>
            <Plus />
            Publish template
          </Button>
        }
      />
      <ChangeNotice key={result.change?.id} change={result.change} />
      {!result.change && result.noOp && (
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
            onUpdate={setUpdating}
          />
        </>
      )}
      {updating && (
        <QuickUpdateDialog
          profileKey={updating}
          onClose={() => setUpdating(null)}
          onAccepted={(change) => {
            setUpdating(null);
            setResult({ change, noOp: change === null });
          }}
        />
      )}
    </>
  );
}
