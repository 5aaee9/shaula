import { Diagnostics } from "@/components/diagnostics";
import { Link, useParams } from "react-router-dom";
import { Button } from "@/components/ui/button";
import { ErrorNotice, Loading } from "@/components/status";
import { jobRepository, useHistory, type JobDetail as Detail } from "@/lib/jobs";
import { ForgejoJobFacts } from "@/components/forgejo-job-details";
import { GitHubJobFacts, JobObservations, RunnerHistory } from "@/components/job-detail-sections";

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
              <p>{jobRepository(job)}</p>
            </div>
            {job.github_run_url && (
              <Button asChild variant="outline">
                <a href={job.github_run_url} target="_blank" rel="noreferrer">
                  Open workflow run
                </a>
              </Button>
            )}
          </div>
          <Diagnostics
            kind="job"
            resourceKey={job.id}
            incarnation={job.fleet_incarnation}
            scopes={scopes}
          />
          {job.forgejo ? <ForgejoJobFacts job={job} /> : <GitHubJobFacts job={job} />}
          <RunnerHistory job={job} scopes={scopes} />
          <JobObservations job={job} />
        </>
      )}
    </>
  );
}
