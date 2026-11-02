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
import { SourceUpdateStatus } from "@/components/source-update-status";

type TemplatesQuery = ReturnType<typeof useTemplates>;

function TemplatesTable({
  items,
  selectedKey,
  canPublish,
  onUpdate,
}: {
  items: TemplateSummary[];
  selectedKey: string | null;
  canPublish: boolean;
  onUpdate: (key: string) => void;
}) {
  const navigate = useNavigate();
  return (
    <div className="table-scroll">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Template profile</TableHead>
            <TableHead>Status</TableHead>
            <TableHead>Default source</TableHead>
            <TableHead>Active revision</TableHead>
            <TableHead>Desired revision</TableHead>
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
              <TableCell>
                <span className="flex flex-wrap items-center gap-2">
                  <span className="text-sm">{item.sourceUpdate?.sourceKey || "--"}</span>
                  {!["Retiring", "Retired"].includes(item.status) && (
                    <SourceUpdateStatus
                      profileKey={item.key}
                      update={item.sourceUpdate}
                      disabled={!canPublish}
                      onUpdate={() => onUpdate(item.key)}
                    />
                  )}
                </span>
              </TableCell>
              <TableCell className="font-mono text-xs">
                {item.activeRevision ? `r${item.activeRevision}` : "--"}
              </TableCell>
              <TableCell className="font-mono text-xs">r{item.desiredRevision}</TableCell>
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
  onUpdate,
}: {
  templates: TemplatesQuery;
  search: string;
  onSearchChange: (value: string) => void;
  selectedKey: string | null;
  canPublish: boolean;
  onUpdate: (key: string) => void;
}) {
  const items =
    templates.data?.data.profiles.filter((item) =>
      item.key.toLowerCase().includes(search.toLowerCase()),
    ) || [];
  return (
    <>
      <div className="section-heading">
        <h2>
          Published templates
          {templates.data && <span className="count">{templates.data.data.profiles.length}</span>}
        </h2>
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
        <TemplatesTable
          items={items}
          selectedKey={selectedKey}
          canPublish={canPublish}
          onUpdate={onUpdate}
        />
      )}
    </>
  );
}
