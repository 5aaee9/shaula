import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";
import { useState } from "react";
import { Layers3, Plus, RefreshCw, Search } from "lucide-react";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";
import { useTemplates } from "@/lib/queries";
import type { ChangeRef } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tip } from "@/components/icon-tooltip";
import { Empty, ErrorNotice, Loading, StatusBadge } from "@/components/status";
import { ChangeNotice } from "@/components/change-notice";
import { TemplateLibrary } from "@/components/template-library";
export function TemplatesPage({ scopes }: { scopes: string[] }) {
  const templates = useTemplates(scopes.includes("template.read"));
  const location = useLocation();
  const navigate = useNavigate();
  const [search, setSearch] = useState("");
  const [params] = useSearchParams();
  const publication = location.state as { change?: ChangeRef | null; noOp?: boolean } | null;
  const [change] = useState<ChangeRef | null>(publication?.change ?? null);
  const key = params.get("key");
  const items =
    templates.data?.data.profiles.filter((item) =>
      item.key.toLowerCase().includes(search.toLowerCase()),
    ) || [];
  // `key` is still honoured in the URL so a redirected detail view can mark
  // its row, but the row no longer expands — the name links to the detail page.
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Templates</h1>
          <p>Create reusable templates and manage their published revisions.</p>
        </div>
        <Button
          onClick={() => navigate("/templates/new")}
          disabled={!scopes.includes("template.publish")}
        >
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
            canPublish={scopes.includes("template.publish")}
            onUse={(source) => navigate("/templates/new", { state: { initialSource: source } })}
          />
          <div className="section-heading">
            <div>
              <h2>
                Published templates
                {templates.data && (
                  <span className="count">{templates.data.data.profiles.length}</span>
                )}
              </h2>
              <p className="mt-1 text-sm text-muted-foreground">
                New revisions activate automatically after validation.
              </p>
            </div>
          </div>
          <div className="toolbar">
            <div className="search-input">
              <Search />
              <Input
                className="pl-9"
                aria-label="Search templates"
                placeholder="Search by template name..."
                value={search}
                onChange={(event) => setSearch(event.target.value)}
              />
            </div>
            <Tip label="Refresh templates">
              <Button
                variant="outline"
                size="icon"
                aria-label="Refresh templates"
                onClick={() => void templates.refetch()}
                disabled={templates.isFetching}
              >
                <RefreshCw className={templates.isFetching ? "animate-spin" : ""} />
              </Button>
            </Tip>
          </div>
          {templates.error ? (
            <ErrorNotice error={templates.error} retry={() => void templates.refetch()} />
          ) : templates.isPending ? (
            <Loading />
          ) : !items.length ? (
            <Empty title={search ? "No matching templates" : "No templates yet"}>
              <p className="max-w-sm text-sm text-muted-foreground">
                {search
                  ? "Try another template name or clear your search to see all templates."
                  : "Choose a default template above, or publish your own template to get started."}
              </p>
              {search && (
                <Button variant="outline" size="sm" onClick={() => setSearch("")}>
                  Clear search
                </Button>
              )}
            </Empty>
          ) : (
            <div className="table-scroll">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Template profile</TableHead>
                    <TableHead>Status</TableHead>
                    <TableHead>Active revision</TableHead>
                    <TableHead>Desired revision</TableHead>
                    <TableHead>
                      <span className="sr-only">Actions</span>
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {items.map((item) => (
                    <TableRow key={item.key} className={key === item.key ? "selected" : ""}>
                      <TableCell>
                        <button
                          className="resource-name text-left"
                          onClick={() => navigate(`/templates/${encodeURIComponent(item.key)}`)}
                        >
                          <span className="resource-icon">
                            <Layers3 />
                          </span>
                          <strong>{item.key}</strong>
                        </button>
                      </TableCell>
                      <TableCell>
                        <StatusBadge value={item.status} />
                      </TableCell>
                      <TableCell className="font-mono text-xs">
                        {item.activeRevision ? `r${item.activeRevision}` : "--"}
                      </TableCell>
                      <TableCell className="font-mono text-xs">r{item.desiredRevision}</TableCell>
                      <TableCell className="text-right">
                        <Button
                          variant="outline"
                          size="sm"
                          disabled={
                            !scopes.includes("template.publish") ||
                            ["Retiring", "Retired"].includes(item.status)
                          }
                          aria-label={`Update ${item.key} from default`}
                          onClick={() =>
                            navigate(`/templates/${encodeURIComponent(item.key)}/update`)
                          }
                        >
                          Update
                        </Button>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </>
      )}
    </>
  );
}
