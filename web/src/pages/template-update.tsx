import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { ArrowLeft } from "lucide-react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { ErrorNotice, Loading } from "@/components/status";
import { TemplateUpdateForm } from "@/components/template-update-form";
import { api, resourcePath } from "@/lib/api";
import type { ChangeRef, TemplateResource, TemplateRevision } from "@/lib/types";

export function TemplateUpdatePage({ scopes }: { scopes: string[] }) {
  const { key: profileKey = "" } = useParams<{ key: string }>();
  const navigate = useNavigate();
  const [reviewId] = useState(() => crypto.randomUUID());
  const canRead = scopes.includes("template.read");
  const canPublish = scopes.includes("template.publish");
  const backTo = `/templates?${new URLSearchParams({ key: profileKey })}`;
  const query = useQuery({
    queryKey: ["template-update", profileKey, reviewId],
    enabled: !!profileKey && canRead && canPublish,
    // Each visit gets a fresh base; background invalidation cannot replace an open review.
    staleTime: "static",
    gcTime: 0,
    queryFn: async ({ signal }) => {
      const path = resourcePath("template-profiles", profileKey);
      const resource = await api<TemplateResource>(path, { signal });
      const revision = await api<TemplateRevision>(
        `${path}/revisions/${resource.data.desiredRevision}`,
        { signal },
      );
      return { resource, revision: revision.data };
    },
  });

  function accepted(change: ChangeRef) {
    navigate(backTo, { replace: true, state: { change } });
  }

  return (
    <div className="mx-auto w-full max-w-5xl">
      <Link className="back-link" to={backTo}>
        <ArrowLeft className="size-4" aria-hidden="true" />
        Back to templates
      </Link>
      <div className="page-heading">
        <div>
          <h1 className="break-words">Update {profileKey} from default</h1>
          <p>Review a default template and publish a new revision with your existing bindings.</p>
        </div>
      </div>
      {!canRead || !canPublish ? (
        <ErrorNotice error={new Error("Template read and publish permissions are required.")} />
      ) : query.isPending ? (
        <Loading />
      ) : query.error ? (
        <ErrorNotice error={query.error} retry={() => void query.refetch()} />
      ) : ["Retiring", "Retired"].includes(query.data.resource.data.status) ? (
        <ErrorNotice error={new Error("A retiring or retired template cannot be updated.")} />
      ) : (
        <TemplateUpdateForm
          key={profileKey}
          resource={query.data.resource}
          revision={query.data.revision}
          onClose={() => navigate(backTo)}
          onAccepted={accepted}
        />
      )}
    </div>
  );
}
