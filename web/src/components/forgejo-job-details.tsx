import type { JobDetail } from "@/lib/jobs";
import { forgejoRunUrl, historyTime, snapshotFreshness } from "@/lib/jobs";
import { forgejoTargetName } from "@/lib/runner-backend";
import { Link } from "react-router-dom";
import { KeyValue, StatusBadge } from "./status";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "./ui/table";

export function ForgejoJobFacts({ job }: { job: JobDetail }) {
  const data = job.forgejo;
  if (!data) return null;
  const runUrl = forgejoRunUrl(job);
  return (
    <section className="details-section">
      <h2>Forgejo workflow job</h2>
      <dl className="details-grid">
        <KeyValue label="Job status">
          <StatusBadge value={job.observed_status} />
        </KeyValue>
        <KeyValue label="Last reported status">{data.last_reported_status}</KeyValue>
        <KeyValue label="Task result">
          {data.result ? <StatusBadge value={data.result.conclusion} /> : "Unknown"}
        </KeyValue>
        {data.result && (
          <>
            <KeyValue label="Result source">
              Repository task history · task {data.result.task_id}
            </KeyValue>
            <KeyValue label="Result observed">{historyTime(data.result.observed_at)}</KeyValue>
            <KeyValue label="Workflow">{data.result.workflow}</KeyValue>
            <KeyValue label="Run number">{data.result.run_number}</KeyValue>
          </>
        )}
        <KeyValue label="Snapshot freshness">{snapshotFreshness(job)}</KeyValue>
        <KeyValue label="Association">{job.association_status}</KeyValue>
        <KeyValue label="Forgejo target">
          <span className="break-all">{forgejoTargetName(data.target)}</span>
        </KeyValue>
        <KeyValue label="Fleet">
          <Link to={`/fleets/${encodeURIComponent(job.fleet_key)}`}>{job.fleet_key}</Link>
        </KeyValue>
        <KeyValue label="Repository ID">{data.repository_id}</KeyValue>
        <KeyValue label="Job ID">{data.job_id}</KeyValue>
        <KeyValue label="Run ID">{data.run_id === "0" ? "Not reported" : data.run_id}</KeyValue>
        <KeyValue label="Job attempt">{data.attempt}</KeyValue>
        <KeyValue label="Last listed">{historyTime(data.last_observed_at)}</KeyValue>
        <KeyValue label="Runs on">{data.runs_on.join(", ")}</KeyValue>
      </dl>
      <p className="mt-4 text-sm text-muted-foreground">
        Forgejo snapshots report waiting and running jobs, not a verified runner assignment.
        {data.result
          ? " The result comes from repository history for the exact observed Task ID. It does not identify which Runner executed the job."
          : " Disappearance does not prove completion or success. Without an exact Task ID, readable task history, or a match within the bounded history window, the result stays unknown."}
      </p>
      {runUrl && (
        <a
          className="mt-3 inline-block underline underline-offset-4"
          href={runUrl}
          target="_blank"
          rel="noreferrer"
        >
          Open workflow run
        </a>
      )}
    </section>
  );
}

export function ForgejoJobObservations({ job }: { job: JobDetail }) {
  return (
    <Table className="table-fixed">
      <TableHeader>
        <TableRow>
          <TableHead className="w-2/5 whitespace-normal">Observed</TableHead>
          <TableHead className="w-2/5 whitespace-normal">Forgejo observation</TableHead>
          <TableHead className="w-1/5 whitespace-normal">Task ID</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {(job.forgejo_observations || []).map((event) => (
          <TableRow key={event.id}>
            <TableCell className="whitespace-normal">{historyTime(event.observed_at)}</TableCell>
            <TableCell className="break-words whitespace-normal">
              {event.source === "task_history"
                ? `Task history: ${event.reported_status}`
                : event.reported_status ||
                  (job.forgejo?.result
                    ? "No longer listed in runner snapshot"
                    : "No longer listed; outcome unknown")}
            </TableCell>
            <TableCell className="break-all whitespace-normal">
              {event.task_id === "0" ? "Not reported" : event.task_id}
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}
