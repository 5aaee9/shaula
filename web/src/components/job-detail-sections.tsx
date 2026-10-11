import { Link } from "react-router-dom";
import { Empty, KeyValue, StatusBadge } from "@/components/status";
import { Table, TableBody, TableCell, TableRow } from "@/components/ui/table";
import { TableColumns } from "@/components/table-columns";
import { historyTime, type JobDetail } from "@/lib/jobs";
import { ForgejoJobObservations } from "@/components/forgejo-job-details";
import { OperationLogs } from "@/components/operation-logs";

export function GitHubJobFacts({ job }: { job: JobDetail }) {
  return (
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
        Status reflects listener observations. An assignment result alone does not establish the
        workflow job’s final conclusion.
      </p>
    </section>
  );
}

export function RunnerHistory({ job, scopes }: { job: JobDetail; scopes: string[] }) {
  return (
    <section className="details-section">
      <h2>Runner history</h2>
      {job.generations.length === 0 ? (
        <Empty title="No verified runner association">
          <p>
            Provisioning history is available in{" "}
            <Link
              className="underline"
              to={`/jobs/runners?fleet_key=${encodeURIComponent(job.fleet_key)}`}
            >
              Unassigned runners
            </Link>
            .
          </p>
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
              {generation.pool_member_key && (
                <span className="text-xs text-muted-foreground">
                  Pool member: {generation.pool_member_key}
                </span>
              )}
              <StatusBadge value={generation.state} />
            </div>
            <OperationLogs generationId={generation.id} scopes={scopes} />
          </div>
        ))
      )}
    </section>
  );
}

export function JobObservations({ job }: { job: JobDetail }) {
  return (
    <section className="details-section">
      <h2>Observations</h2>
      {job.observations_truncated && (
        <p className="mb-3 text-sm text-muted-foreground">Showing the latest 1,000 observations.</p>
      )}
      {job.forgejo ? (
        <ForgejoJobObservations job={job} />
      ) : (
        <Table>
          <TableColumns
            labels={["Observed", "Event", "Request", "Runner", "Reported result", "Association"]}
          />
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
      )}
    </section>
  );
}
