import { Link, useSearchParams } from "react-router-dom";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { Empty, ErrorNotice, Loading, StatusBadge } from "@/components/status";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { historyTime, useHistory, type Job, type Page } from "@/lib/jobs";

export function JobsPage({ scopes }: { scopes: string[] }) {
  const [params, setParams] = useSearchParams();
  const permitted = scopes.includes("fleet.read");
  const jobs = useHistory<Page<Job>>(`/jobs?${params}`, permitted);
  function filter(key: string, value: string) {
    const next = new URLSearchParams(params);
    next.delete("cursor");
    if (value) next.set(key, value);
    else next.delete(key);
    setParams(next, { replace: true });
  }
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Jobs</h1>
          <p>Workflow jobs observed by Shaula, with runner and provisioning history.</p>
        </div>
        <Button asChild variant="outline">
          <Link to="/jobs/runners">Unassigned runners</Link>
        </Button>
      </div>
      {!permitted ? (
        <ErrorNotice error={new Error("fleet.read permission is required.")} />
      ) : (
        <>
          <div className="toolbar flex-wrap">
            <Input
              aria-label="Filter by fleet"
              placeholder="Fleet"
              className="w-40"
              value={params.get("fleet_key") || ""}
              onChange={(e) => filter("fleet_key", e.target.value)}
            />
            <Input
              aria-label="Filter by repository"
              placeholder="Repository"
              className="w-48"
              value={params.get("repository") || ""}
              onChange={(e) => filter("repository", e.target.value)}
            />
            <Input
              aria-label="Filter by job name"
              placeholder="Job name"
              className="w-48"
              value={params.get("job_name") || ""}
              onChange={(e) => filter("job_name", e.target.value)}
            />
            <NativeSelect
              aria-label="Filter by job status"
              value={params.get("status") || ""}
              onChange={(e) => filter("status", e.target.value)}
            >
              <NativeSelectOption value="">All statuses</NativeSelectOption>
              {[
                "queued",
                "assigned",
                "running",
                "completed",
                "assignment_withdrawn",
                "unknown",
              ].map((status) => (
                <NativeSelectOption key={status} value={status}>
                  {status.replaceAll("_", " ")}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </div>
          {jobs.error && <ErrorNotice error={jobs.error} retry={() => void jobs.refetch()} />}
          {jobs.isPending ? (
            <Loading />
          ) : (
            jobs.data && (
              <>
                {jobs.data.items.length === 0 ? (
                  <Empty title="No observed jobs">
                    Jobs appear when the Scale Set listener observes them. Runners without a
                    verified job association are available separately.
                  </Empty>
                ) : (
                  <div className="rounded-md border">
                    <Table>
                      <TableHeader>
                        <TableRow>
                          <TableHead>Workflow job</TableHead>
                          <TableHead>Repository / run</TableHead>
                          <TableHead>Fleet</TableHead>
                          <TableHead>Job status</TableHead>
                          <TableHead>Reported result</TableHead>
                          <TableHead>Updated</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {jobs.data.items.map((job) => (
                          <TableRow key={job.id}>
                            <TableCell>
                              <Link
                                className="font-medium underline-offset-4 hover:underline"
                                to={`/jobs/${encodeURIComponent(job.id)}`}
                              >
                                {job.job_display_name || job.protocol_job_id}
                              </Link>
                              <p className="text-xs text-muted-foreground">
                                {job.association_status} association
                              </p>
                            </TableCell>
                            <TableCell>
                              {[job.owner_name, job.repository_name].filter(Boolean).join("/") ||
                                "Unknown"}
                              <p className="text-xs text-muted-foreground">
                                {job.workflow_run_id ? `Run ${job.workflow_run_id}` : "Run unknown"}
                              </p>
                            </TableCell>
                            <TableCell>
                              <Link to={`/fleets/${encodeURIComponent(job.fleet_key)}`}>
                                {job.fleet_key}
                              </Link>
                            </TableCell>
                            <TableCell>
                              <StatusBadge value={job.observed_status} />
                            </TableCell>
                            <TableCell>{job.reported_result || "Unknown"}</TableCell>
                            <TableCell>{historyTime(job.updated_at)}</TableCell>
                          </TableRow>
                        ))}
                      </TableBody>
                    </Table>
                  </div>
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
                    disabled={!jobs.data.next_cursor}
                    onClick={() => filter("cursor", jobs.data?.next_cursor || "")}
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
