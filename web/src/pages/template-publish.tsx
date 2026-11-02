import { PageHeader } from "@/components/page-header";
import { useQuery } from "@tanstack/react-query";
import { useLocation, useNavigate, useParams } from "react-router-dom";
import { TemplatePublishGate } from "@/components/template-publish-gate";
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
  const recorded = query.data?.data.sourceUpdate;
  // A new revision keeps its recorded default source unless the publisher switches.
  const initialSource = !canRead
    ? undefined
    : !profileKey
      ? (location.state as { initialSource?: TemplateSource } | null)?.initialSource
      : recorded?.targetDigest && recorded.targetEngine && query.data?.data.platform
        ? {
            key: recorded.sourceKey,
            artifactDigest: recorded.targetDigest,
            engineRef: recorded.targetEngine,
            platform: query.data.data.platform,
          }
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
      <PageHeader
        back={{ to: backTo, label: "Templates" }}
        title={profileKey ? `Publish revision of ${profileKey}` : "Publish template"}
      />
      <TemplatePublishGate
        profileKey={profileKey}
        canPublish={canPublish}
        canRead={canRead}
        profile={query}
      >
        <TemplateForm
          key={location.key}
          resource={profileKey ? query.data : undefined}
          initialSource={initialSource}
          scopes={scopes}
          onClose={() => navigate(backTo, { replace: true })}
          onAccepted={accepted}
        />
      </TemplatePublishGate>
    </div>
  );
}
