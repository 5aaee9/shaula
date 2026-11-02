import { PageHeader } from "@/components/page-header";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { api, resourcePath } from "@/lib/api";
import type { TemplateResource } from "@/lib/types";
import { useTemplateRevision } from "@/lib/use-template-revision";
import { ErrorNotice, Loading, StatusBadge } from "@/components/status";
import { RetireDialog } from "@/components/retire-dialog";
import {
  TemplateDetailActions,
  TemplateProfileSummary,
  TemplateRevisionSection,
} from "@/components/template-detail-sections";

export function TemplateDetailPage({ scopes }: { scopes: string[] }) {
  const { key: profileKey = "" } = useParams<{ key: string }>();
  const navigate = useNavigate();
  const canRead = scopes.includes("template.read");
  const path = resourcePath("template-profiles", profileKey);
  const backTo = `/templates?${new URLSearchParams({ key: profileKey })}`;

  const query = useQuery({
    queryKey: ["template", profileKey],
    enabled: !!profileKey && canRead,
    queryFn: ({ signal }) => api<TemplateResource>(path, { signal }),
    refetchInterval: 10000,
  });
  const selection = useTemplateRevision({
    path,
    profileKey,
    desired: query.data?.data.desiredRevision,
    enabled: canRead,
  });
  const [retire, setRetire] = useState(false);

  if (!canRead) return <ErrorNotice error={new Error("Template read permission is required.")} />;
  if (query.isPending) return <Loading />;
  if (query.error) return <ErrorNotice error={query.error} retry={() => void query.refetch()} />;
  const { data } = query.data;

  return (
    <div className="mx-auto w-full max-w-5xl">
      <PageHeader
        back={{ to: backTo, label: "Templates" }}
        title={profileKey}
        meta={
          <>
            <StatusBadge value={data.status} />
            {data.platform && <span>{data.platform}</span>}
          </>
        }
        actions={
          <TemplateDetailActions
            profileKey={profileKey}
            canPublish={scopes.includes("template.publish")}
            canRetire={scopes.includes("template.retire")}
            retired={["Retiring", "Retired"].includes(data.status)}
            onRetire={() => setRetire(true)}
          />
        }
      />

      <TemplateProfileSummary data={data} />
      <TemplateRevisionSection
        revision={selection.revision}
        onRevisionChange={selection.setSelected}
        error={selection.query.error}
        data={selection.query.data?.data}
      />

      {retire && (
        <RetireDialog
          name={profileKey}
          path={path}
          etag={query.data.etag}
          type="profile"
          onClose={() => setRetire(false)}
          onAccepted={() => navigate("/templates", { replace: true })}
        />
      )}
    </div>
  );
}
