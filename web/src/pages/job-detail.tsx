import { Link, useParams } from "react-router-dom";
import { Button } from "@/components/ui/button";
import { Empty, ErrorNotice, KeyValue, Loading, StatusBadge } from "@/components/status";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { historyTime, useHistory, type JobDetail as Detail } from "@/lib/jobs";
import { OperationLogs } from "@/components/operation-logs";

export function JobDetail({ scopes }: { scopes: string[] }) {
  const { id = "" } = useParams();
  const permitted = scopes.includes("fleet.read");
  const detail = useHistory<Detail>(`/jobs/${encodeURIComponent(id)}`, permitted);
  if (!permitted) return <ErrorNotice error={new Error("fleet.read permission is required.")} />;
  if (detail.isPending) return <Loading />;
  const job = detail.data;
  return (
    <>
      <Link to="/jobs" className="back-link">
        ← Jobs
      </Link>
      {detail.error && <ErrorNotice error={detail.error} retry={() => void detail.refetch()} />}
      {job && (
        <>
          <div className="page-heading">
            <div>
              <h1>{job.job_display_name || job.protocol_job_id}</h1>
              <p>
                {[job.owner_name, job.repository_name].filter(Boolean).join("/") ||
                  "Repository unknown"}
              </p>
            </div>
            {job.github_run_url && (
              <Button asChild variant="outline">
                <a href={job.github_run_url} target="_blank" rel="noreferrer">
                  Open workflow run
                </a>
              </Button>
            )}
          </div>
          <section className="details-section">
            <h2>Workflow job</h2>
            <dl className="details-grid">
              <KeyValue label="Job status">
                <StatusBadge value={job.observed_status} />
              </KeyValue>
              <KeyValue label="Reported result">{job.reported_result || "Unknown"}</KeyValue>
              <KeyValue label="GitHub conclusion">{job.github_conclusion || "Unknown"}</KeyValue>
              <KeyValue label="Fleet">
                <Link to={`/fleets/${encodeURIComponent(job.fleet_key)}`}>{job.fleet_key}</Link>
              </KeyValue>
              <KeyValue label="Workflow run">{job.workflow_run_id || "Unknown"}</KeyValue>
              <KeyValue label="Run attempt">{job.workflow_run_attempt || "Unknown"}</KeyValue>
              <KeyValue label="Workflow">{job.job_workflow_ref || "Unknown"}</KeyValue>
              <KeyValue label="Listener freshness">{job.freshness}</KeyValue>
              <KeyValue label="Association">{job.association_status}</KeyValue>
              <KeyValue label="Last observed">{historyTime(job.updated_at)}</KeyValue>
            </dl>
            <p className="mt-4 text-sm text-muted-foreground">
              Status reflects listener observations. An assignment result alone does not establish
              the workflow job’s final conclusion.
            </p>
          </section>
          <section className="details-section">
            <h2>Runner history</h2>
            {job.generations.length === 0 ? (
              <Empty title="No verified runner association">
                Provisioning history is available in{" "}
                <Link
                  className="underline"
                  to={`/jobs/runners?fleet_key=${encodeURIComponent(job.fleet_key)}`}
                >
                  Unassigned runners
                </Link>
                .
              </Empty>
            ) : (
              job.generations.map((generation) => (
                <div key={generation.id} className="mt-4 rounded-md border p-4">
                  <div className="flex flex-wrap items-center justify-between gap-3">
                    <Link
                      className="font-medium underline"
                      to={`/jobs/runners/${encodeURIComponent(generation.id)}`}
                    >
                      {generation.runner_name}
                    </Link>
                    <StatusBadge value={generation.state} />
                  </div>
                  <OperationLogs generationId={generation.id} scopes={scopes} />
                </div>
              ))
            )}
          </section>
          <section className="details-section">
            <h2>Observations</h2>
            {job.observations_truncated && (
              <p className="mb-3 text-sm text-muted-foreground">
                Showing the latest 1,000 observations.
              </p>
            )}
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Observed</TableHead>
                  <TableHead>Event</TableHead>
                  <TableHead>Request</TableHead>
                  <TableHead>Runner</TableHead>
                  <TableHead>Reported result</TableHead>
                  <TableHead>Association</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {job.observations.map((observation) => (
                  <TableRow key={observation.id}>
                    <TableCell>{historyTime(observation.observed_at)}</TableCell>
                    <TableCell>{observation.kind}</TableCell>
                    <TableCell>{observation.runner_request_id}</TableCell>
                    <TableCell>
                      {observation.runner_name || observation.runner_id || "Unknown"}
                    </TableCell>
                    <TableCell>{observation.reported_result || "Unknown"}</TableCell>
                    <TableCell>{observation.association_status}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </section>
        </>
      )}
    </>
  );
}
