import type { JobDetail } from "@/lib/jobs";
import { gitlabJobUrl, historyTime, snapshotFreshness } from "@/lib/jobs";
import { gitlabTargetName } from "@/lib/runner-backend";
import { Link } from "react-router-dom";
import { KeyValue, StatusBadge } from "./status";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "./ui/table";

export function GitlabJobFacts({ job }: { job: JobDetail }) {
  const data = job.gitlab;
  if (!data) return null;
  const jobUrl = gitlabJobUrl(job);
  return (
    <section className="details-section">
      <h2>GitLab CI job</h2>
      <dl className="details-grid">
        <KeyValue label="Job status">
          <StatusBadge value={job.observed_status} />
        </KeyValue>
        <KeyValue label="Last reported status">{data.last_reported_status}</KeyValue>
        <KeyValue label="Job result">
          {data.result ? <StatusBadge value={data.result.status} /> : "Unknown"}
        </KeyValue>
        {data.result && (
          <KeyValue label="Result observed">{historyTime(data.result.observed_at)}</KeyValue>
        )}
        <KeyValue label="Read freshness">{snapshotFreshness(job)}</KeyValue>
        <KeyValue label="Association">{job.association_status}</KeyValue>
        <KeyValue label="GitLab target">
          <span className="break-all">{gitlabTargetName(data.target)}</span>
        </KeyValue>
        <KeyValue label="Fleet">
          <Link to={`/fleets/${encodeURIComponent(job.fleet_key)}`}>{job.fleet_key}</Link>
        </KeyValue>
        <KeyValue label="Project">
          <span className="break-all">{data.project_path}</span>
        </KeyValue>
        <KeyValue label="Project ID">{data.project_id}</KeyValue>
        <KeyValue label="Pipeline">{data.pipeline_id}</KeyValue>
        <KeyValue label="Job ID">{data.job_id}</KeyValue>
        <KeyValue label="Last observed">{historyTime(data.last_observed_at)}</KeyValue>
      </dl>
      <p className="mt-4 text-sm text-muted-foreground">
        The association is Verified: this job ran on the registration of a Shaula Runner, recorded
        before that registration was paused. GitLab clears the job’s runner once the registration is
        deleted; the recorded association remains.
        {data.result
          ? " The result is the job’s own terminal status."
          : " Without a read of the job’s own terminal status, the result stays unknown."}
      </p>
      {jobUrl && (
        <a
          className="mt-3 inline-block underline underline-offset-4"
          href={jobUrl}
          target="_blank"
          rel="noreferrer"
        >
          Open GitLab job
        </a>
      )}
    </section>
  );
}

export function GitlabJobObservations({ job }: { job: JobDetail }) {
  return (
    <Table className="table-fixed">
      <TableHeader>
        <TableRow>
          <TableHead className="w-2/5 whitespace-normal">Observed</TableHead>
          <TableHead className="w-2/5 whitespace-normal">GitLab observation</TableHead>
          <TableHead className="w-1/5 whitespace-normal">Runner ID</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {(job.gitlab_observations || []).map((event) => (
          <TableRow key={event.id}>
            <TableCell className="whitespace-normal">{historyTime(event.observed_at)}</TableCell>
            <TableCell className="break-words whitespace-normal">
              {event.reported_status === null
                ? "Job no longer found; outcome unknown"
                : event.source === "runner_jobs"
                  ? `Runner jobs: ${event.reported_status}`
                  : `Job read: ${event.reported_status}`}
            </TableCell>
            <TableCell className="break-all whitespace-normal">
              {event.runner_id || "Not reported"}
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}
