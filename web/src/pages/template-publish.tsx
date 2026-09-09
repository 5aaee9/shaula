import { useQuery } from "@tanstack/react-query";
import { ArrowLeft } from "lucide-react";
import { Link, useLocation, useNavigate, useParams } from "react-router-dom";
import { ErrorNotice, Loading } from "@/components/status";
import { TemplateForm } from "@/components/template-form";
import { api, resourcePath } from "@/lib/api";
import type { TemplateSource } from "@/lib/template-variables";
import type { ChangeRef, TemplateResource } from "@/lib/types";

export function TemplatePublishPage({ scopes }: { scopes: string[] }) {
  const { key: profileKey } = useParams<{ key: string }>();
  const location = useLocation();
  const navigate = useNavigate();
  const canPublish = scopes.includes("template.publish");
  const canRead = scopes.includes("template.read");
  const query = useQuery({
    queryKey: ["template", profileKey],
    enabled: !!profileKey && canPublish && canRead,
    queryFn: ({ signal }) =>
      api<TemplateResource>(resourcePath("template-profiles", profileKey!), { signal }),
  });
  const initialSource =
    !profileKey && canRead
      ? (location.state as { initialSource?: TemplateSource } | null)?.initialSource
      : undefined;
  const backTo = profileKey
    ? `/templates?${new URLSearchParams({ key: profileKey })}`
    : "/templates";

  function accepted(change: ChangeRef | null) {
    navigate(change ? `/templates?${new URLSearchParams({ key: change.resource })}` : backTo, {
      replace: true,
      state: { change, noOp: change === null },
    });
  }

  return (
    <div className="mx-auto w-full max-w-5xl">
      <Link className="back-link" to={backTo}>
        <ArrowLeft className="size-4" aria-hidden="true" />
        Back to templates
      </Link>
      <div className="page-heading">
        <div>
          <h1 className="break-all">
            {profileKey ? `Publish revision of ${profileKey}` : "Publish template"}
          </h1>
          <p>
            {profileKey
              ? "Configure a new revision. It will activate automatically after validation."
              : "Choose a source and configure a reusable template for your fleets."}
          </p>
        </div>
      </div>
      {!canPublish ? (
        <ErrorNotice error={new Error("Template publish permission is required.")} />
      ) : profileKey && !canRead ? (
        <ErrorNotice
          error={new Error("Template read permission is required to publish a revision.")}
        />
      ) : profileKey && query.isPending ? (
        <Loading />
      ) : profileKey && query.error ? (
        <ErrorNotice error={query.error} retry={() => void query.refetch()} />
      ) : (
        <TemplateForm
          key={location.key}
          resource={profileKey ? query.data : undefined}
          initialSource={initialSource}
          scopes={scopes}
          onClose={() => navigate(backTo, { replace: true })}
          onAccepted={accepted}
        />
      )}
    </div>
  );
}
