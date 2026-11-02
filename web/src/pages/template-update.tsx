import { PageHeader } from "@/components/page-header";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { ErrorNotice, Loading } from "@/components/status";
import { TemplateUpdateForm } from "@/components/template-update-form";
import { api, resourcePath } from "@/lib/api";
import type { TemplateSource } from "@/lib/template-variables";
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
    // Each visit freezes its base and catalog; background invalidation cannot replace the review.
    staleTime: "static",
    gcTime: 0,
    queryFn: async ({ signal }) => {
      const path = resourcePath("template-profiles", profileKey);
      const resource = await api<TemplateResource>(path, { signal });
      const [revision, catalog] = await Promise.all([
        api<TemplateRevision>(`${path}/revisions/${resource.data.desiredRevision}`, { signal }),
        api<{ sources: TemplateSource[] }>("/template-sources", { signal }),
      ]);
      return { resource, revision: revision.data, sources: catalog.data.sources };
    },
  });

  function accepted(change: ChangeRef | null) {
    navigate(backTo, { replace: true, state: { change, noOp: change === null } });
  }

  return (
    <div className="mx-auto w-full max-w-5xl">
      <PageHeader
        back={{ to: backTo, label: "Templates" }}
        title={`Update ${profileKey} from default`}
      />
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
          sources={query.data.sources}
          onClose={() => navigate(backTo)}
          onAccepted={accepted}
        />
      )}
    </div>
  );
}
