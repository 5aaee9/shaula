import type { ReactNode } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import { ErrorNotice, Loading } from "@/components/status";

/**
 * Renders the publish form only once permissions allow it and, for a new
 * revision, the existing profile has loaded.
 */
export function TemplatePublishGate({
  profileKey,
  canPublish,
  canRead,
  profile,
  children,
}: {
  profileKey: string | undefined;
  canPublish: boolean;
  canRead: boolean;
  profile: Pick<UseQueryResult, "isPending" | "error" | "refetch">;
  children: ReactNode;
}) {
  if (!canPublish)
    return <ErrorNotice error={new Error("Template publish permission is required.")} />;
  if (!profileKey) return children;
  if (!canRead)
    return (
      <ErrorNotice
        error={new Error("Template read permission is required to publish a revision.")}
      />
    );
  if (profile.isPending) return <Loading />;
  if (profile.error)
    return <ErrorNotice error={profile.error} retry={() => void profile.refetch()} />;
  return children;
}
