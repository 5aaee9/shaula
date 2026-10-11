import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { ArrowLeft } from "lucide-react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { api, resourcePath } from "@/lib/api";
import type { TemplateResource } from "@/lib/types";
import { useTemplateRevision } from "@/lib/use-template-revision";
import { ErrorNotice, Loading } from "@/components/status";
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
      <Link className="back-link" to={backTo}>
        <ArrowLeft className="size-4" aria-hidden="true" />
        Back to templates
      </Link>
      <div className="page-heading">
        <div>
          <h1 className="break-all">{profileKey}</h1>
          <p>Template profile detail and published revisions.</p>
        </div>
        <TemplateDetailActions
          profileKey={profileKey}
          canPublish={scopes.includes("template.publish")}
          canRetire={scopes.includes("template.retire")}
          retired={["Retiring", "Retired"].includes(data.status)}
          onRetire={() => setRetire(true)}
        />
      </div>

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
