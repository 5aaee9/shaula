import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Layers3, RefreshCw, Search, Trash2, Upload } from "lucide-react";
import { useSearchParams } from "react-router-dom";
import { api, resourcePath } from "@/lib/api";
import { useTemplates } from "@/lib/queries";
import type { ChangeRef, TemplateResource, TemplateRevision } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tip } from "@/components/icon-tooltip";
import { Empty, ErrorNotice, KeyValue, Loading, StatusBadge } from "@/components/status";
import { ChangeNotice } from "@/components/change-notice";
import { RetireDialog } from "@/components/retire-dialog";
import { TemplateForm } from "@/components/template-form";
export function TemplatesPage({ scopes }: { scopes: string[] }) {
  const templates = useTemplates(scopes.includes("template.read"));
  const [search, setSearch] = useState("");
  const [params, setParams] = useSearchParams();
  const [publish, setPublish] = useState(false);
  const [change, setChange] = useState<ChangeRef | null>(null);
  const key = params.get("key");
  const items =
    templates.data?.data.profiles.filter((item) =>
      item.key.toLowerCase().includes(search.toLowerCase()),
    ) || [];
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Templates</h1>
          <p>Immutable template revisions and their activation status.</p>
        </div>
        <Button onClick={() => setPublish(true)} disabled={!scopes.includes("template.publish")}>
          <Upload />
          Publish template
        </Button>
      </div>
      <ChangeNotice change={change} />
      {!scopes.includes("template.read") ? (
        <ErrorNotice error={new Error("Template read permission is required.")} />
      ) : (
        <>
          <div className="toolbar">
            <div className="search-input">
              <Search />
              <Input
                className="pl-9"
                aria-label="Search templates"
                placeholder="Search templates..."
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
              >
                <RefreshCw />
              </Button>
            </Tip>
          </div>
          {templates.error ? (
            <ErrorNotice error={templates.error} retry={() => void templates.refetch()} />
          ) : templates.isPending ? (
            <Loading />
          ) : !items.length ? (
            <Empty title={search ? "No matching templates" : "No templates yet"} />
          ) : (
            <div className="table-scroll">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Template profile</TableHead>
                    <TableHead>Status</TableHead>
                    <TableHead>Active revision</TableHead>
                    <TableHead>Desired revision</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {items.map((item) => (
                    <TableRow key={item.key} className={key === item.key ? "selected" : ""}>
                      <TableCell>
                        <button
                          className="resource-name text-left"
                          onClick={() => setParams({ key: item.key })}
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
                        {item.activeRevision ? `r${item.activeRevision}` : "--"}
                      </TableCell>
                      <TableCell>r{item.desiredRevision}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
          {key && (
            <TemplateDetails key={key} profileKey={key} scopes={scopes} onAccepted={setChange} />
          )}
        </>
      )}
      {publish && <TemplateForm onClose={() => setPublish(false)} onAccepted={setChange} />}
    </>
  );
}
function TemplateDetails({
  profileKey,
  scopes,
  onAccepted,
}: {
  profileKey: string;
  scopes: string[];
  onAccepted: (change: ChangeRef) => void;
}) {
  const path = resourcePath("template-profiles", profileKey);
  const query = useQuery({
    queryKey: ["template", profileKey],
    queryFn: ({ signal }) => api<TemplateResource>(path, { signal }),
    refetchInterval: 10000,
  });
  const [selected, setSelected] = useState("");
  const desired = query.data?.data.desiredRevision;
  const revision = selected || (desired ? String(desired) : "");
  const revisionQuery = useQuery({
    queryKey: ["template-revision", profileKey, revision],
    enabled: !!revision,
    queryFn: ({ signal }) =>
      api<TemplateRevision>(`${path}/revisions/${encodeURIComponent(revision)}`, { signal }),
    refetchInterval: 10000,
  });
  const [retire, setRetire] = useState(false);
  const [publish, setPublish] = useState(false);
  if (query.isPending) return <Loading />;
  if (query.error) return <ErrorNotice error={query.error} retry={() => void query.refetch()} />;
  const { data } = query.data;
  return (
    <section className="details-section">
      <div className="section-heading">
        <h2 className="break-all">{profileKey}</h2>
        <div className="flex gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={!scopes.includes("template.publish")}
            onClick={() => setPublish(true)}
          >
            <Upload />
            New revision
          </Button>
          <Button
            variant="outline"
            size="icon"
            disabled={!scopes.includes("template.retire")}
            aria-label="Retire template"
            onClick={() => setRetire(true)}
          >
            <Trash2 />
          </Button>
        </div>
      </div>
      <dl className="details-grid">
        <KeyValue label="Platform">{data.platform || "--"}</KeyValue>
        <KeyValue label="Bindings contract">{data.bindingsContract || "--"}</KeyValue>
        <KeyValue label="Bindings">{data.bindings_present ? "Configured" : "Absent"}</KeyValue>
        <KeyValue label="Status">
          <StatusBadge value={data.status} />
        </KeyValue>
      </dl>
      <div className="mt-6 flex items-center gap-3">
        <h3 className="text-sm font-medium">Revision</h3>
        <Input
          className="w-24"
          aria-label="Template revision"
          type="number"
          min={1}
          step={1}
          value={revision}
          onChange={(event) => setSelected(event.target.value)}
        />
      </div>
      {revisionQuery.error ? (
        <ErrorNotice error={revisionQuery.error} />
      ) : revisionQuery.data ? (
        <dl className="details-grid mt-4">
          <KeyValue label="Artifact digest">
            <span className="mono text-xs">{revisionQuery.data.data.artifactDigest}</span>
          </KeyValue>
          <KeyValue label="Engine">{revisionQuery.data.data.engineRef}</KeyValue>
          <KeyValue label="Revision status">
            <StatusBadge value={revisionQuery.data.data.state} />
          </KeyValue>
        </dl>
      ) : (
        <Loading />
      )}
      {retire && (
        <RetireDialog
          name={profileKey}
          path={path}
          etag={query.data.etag}
          type="profile"
          onClose={() => setRetire(false)}
          onAccepted={onAccepted}
        />
      )}
      {publish && (
        <TemplateForm
          resource={query.data}
          onClose={() => setPublish(false)}
          onAccepted={onAccepted}
        />
      )}
    </section>
  );
}
