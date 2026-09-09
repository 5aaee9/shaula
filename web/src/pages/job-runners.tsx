import { Link, useParams, useSearchParams } from "react-router-dom";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Empty, ErrorNotice, KeyValue, Loading, StatusBadge } from "@/components/status";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { OperationLogs } from "@/components/operation-logs";
import {
  historyTime,
  useHistory,
  type Generation,
  type GenerationDetail,
  type Page,
} from "@/lib/jobs";

export function UnassignedRunners({ scopes }: { scopes: string[] }) {
  const [params, setParams] = useSearchParams();
  const query = new URLSearchParams(params);
  query.set("association", "unassigned");
  const permitted = scopes.includes("fleet.read");
  const generations = useHistory<Page<Generation>>(`/generations?${query}`, permitted);
  function filter(key: string, value: string) {
    const next = new URLSearchParams(params);
    next.delete("cursor");
    if (value) next.set(key, value);
    else next.delete(key);
    setParams(next, { replace: true });
  }
  return (
    <>
      <Link className="back-link" to="/jobs">
        ← Jobs
      </Link>
      <div className="page-heading">
        <div>
          <h1>Unassigned runners</h1>
          <p>Warm, failed, or historical runners without a verified workflow job association.</p>
        </div>
      </div>
      {!permitted ? (
        <ErrorNotice error={new Error("fleet.read permission is required.")} />
      ) : (
        <>
          <div className="toolbar">
            <Input
              className="w-48"
              aria-label="Filter by fleet"
              placeholder="Fleet"
              value={params.get("fleet_key") || ""}
              onChange={(e) => filter("fleet_key", e.target.value)}
            />
          </div>
          {generations.error && (
            <ErrorNotice error={generations.error} retry={() => void generations.refetch()} />
          )}
          {generations.isPending ? (
            <Loading />
          ) : (
            generations.data && (
              <>
                {generations.data.items.length === 0 ? (
                  <Empty title="No unassigned runners" />
                ) : (
                  <Table>
                    <TableHeader>
                      <TableRow>
                        <TableHead>Runner</TableHead>
                        <TableHead>Fleet</TableHead>
                        <TableHead>Runner status</TableHead>
                        <TableHead>Association</TableHead>
                        <TableHead>Updated</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {generations.data.items.map((generation) => (
                        <TableRow key={generation.id}>
                          <TableCell>
                            <Link
                              className="font-medium underline"
                              to={`/jobs/runners/${encodeURIComponent(generation.id)}`}
                            >
                              {generation.runner_name}
                            </Link>
                          </TableCell>
                          <TableCell>{generation.fleet_key}</TableCell>
                          <TableCell>
                            <StatusBadge value={generation.state} />
                          </TableCell>
                          <TableCell>{generation.association_status}</TableCell>
                          <TableCell>{historyTime(generation.updated_at)}</TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                )}
                <div className="mt-4 flex gap-2">
                  <Button
                    variant="outline"
                    disabled={!params.has("cursor")}
                    onClick={() => filter("cursor", "")}
                  >
                    First page
                  </Button>
                  <Button
                    variant="outline"
                    disabled={!generations.data.next_cursor}
                    onClick={() => filter("cursor", generations.data?.next_cursor || "")}
                  >
                    Next page
                  </Button>
                </div>
              </>
            )
          )}
        </>
      )}
    </>
  );
}

export function RunnerDetail({ scopes }: { scopes: string[] }) {
  const { id = "" } = useParams();
  const permitted = scopes.includes("fleet.read");
  const detail = useHistory<GenerationDetail>(`/generations/${encodeURIComponent(id)}`, permitted);
  if (!permitted) return <ErrorNotice error={new Error("fleet.read permission is required.")} />;
  if (detail.isPending) return <Loading />;
  const generation = detail.data;
  return (
    <>
      <Link className="back-link" to="/jobs/runners">
        ← Unassigned runners
      </Link>
      {detail.error && <ErrorNotice error={detail.error} retry={() => void detail.refetch()} />}
      {generation && (
        <>
          <div className="page-heading">
            <div>
              <h1>{generation.runner_name}</h1>
              <p>Runner generation and retained provisioning history.</p>
            </div>
          </div>
          <section className="details-section">
            <h2>Runner</h2>
            <dl className="details-grid">
              <KeyValue label="Status">
                <StatusBadge value={generation.state} />
              </KeyValue>
              <KeyValue label="Phase">{generation.subphase || "Unknown"}</KeyValue>
              <KeyValue label="Fleet">
                <Link to={`/fleets/${encodeURIComponent(generation.fleet_key)}`}>
                  {generation.fleet_key}
                </Link>
              </KeyValue>
              <KeyValue label="GitHub runner ID">
                {generation.github_runner_id || "Unknown"}
              </KeyValue>
              <KeyValue label="Generation">{generation.id}</KeyValue>
              <KeyValue label="Job association">
                <StatusBadge value={generation.association_status} />
              </KeyValue>
              <KeyValue label="Created">{historyTime(generation.created_at)}</KeyValue>
            </dl>
          </section>
          <section className="details-section">
            <h2>Workflow jobs</h2>
            {generation.association_status !== "verified" && generation.jobs.length > 0 && (
              <p className="mb-3 text-sm text-muted-foreground">
                These are candidate jobs with conflicting association evidence. No workflow job is
                confirmed for this runner.
              </p>
            )}
            {generation.jobs.length ? (
              generation.jobs.map((job) => (
                <p key={job.id}>
                  <Link className="underline" to={`/jobs/${encodeURIComponent(job.id)}`}>
                    {job.job_display_name || job.protocol_job_id}
                  </Link>{" "}
                  <StatusBadge value={job.observed_status} />
                </p>
              ))
            ) : (
              <p className="text-sm text-muted-foreground">No verified job association.</p>
            )}
          </section>
          <OperationLogs generationId={generation.id} scopes={scopes} />
        </>
      )}
    </>
  );
}
