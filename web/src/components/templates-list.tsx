import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";
import { Layers3, RefreshCw, Search } from "lucide-react";
import { useNavigate } from "react-router-dom";
import type { useTemplates } from "@/lib/queries";
import type { TemplateSummary } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tip } from "@/components/icon-tooltip";
import { Empty, ErrorNotice, Loading, StatusBadge } from "@/components/status";

type TemplatesQuery = ReturnType<typeof useTemplates>;

function TemplatesTable({
  items,
  selectedKey,
  canPublish,
}: {
  items: TemplateSummary[];
  selectedKey: string | null;
  canPublish: boolean;
}) {
  const navigate = useNavigate();
  return (
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
            <TableRow key={item.key} className={selectedKey === item.key ? "selected" : ""}>
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
                  disabled={!canPublish || ["Retiring", "Retired"].includes(item.status)}
                  aria-label={`Update ${item.key} from default`}
                  onClick={() => navigate(`/templates/${encodeURIComponent(item.key)}/update`)}
                >
                  Update
                </Button>
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  );
}

function NoTemplates({ search, onClearSearch }: { search: string; onClearSearch: () => void }) {
  return (
    <Empty title={search ? "No matching templates" : "No templates yet"}>
      <p className="max-w-sm text-sm text-muted-foreground">
        {search
          ? "Try another template name or clear your search to see all templates."
          : "Choose a default template above, or publish your own template to get started."}
      </p>
      {search && (
        <Button variant="outline" size="sm" onClick={onClearSearch}>
          Clear search
        </Button>
      )}
    </Empty>
  );
}

export function PublishedTemplates({
  templates,
  search,
  onSearchChange,
  selectedKey,
  canPublish,
}: {
  templates: TemplatesQuery;
  search: string;
  onSearchChange: (value: string) => void;
  selectedKey: string | null;
  canPublish: boolean;
}) {
  const items =
    templates.data?.data.profiles.filter((item) =>
      item.key.toLowerCase().includes(search.toLowerCase()),
    ) || [];
  return (
    <>
      <div className="section-heading">
        <div>
          <h2>
            Published templates
            {templates.data && <span className="count">{templates.data.data.profiles.length}</span>}
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
            onChange={(event) => onSearchChange(event.target.value)}
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
        <NoTemplates search={search} onClearSearch={() => onSearchChange("")} />
      ) : (
        <TemplatesTable items={items} selectedKey={selectedKey} canPublish={canPublish} />
      )}
    </>
  );
}
