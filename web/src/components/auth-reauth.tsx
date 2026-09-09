import { useEffect, useRef, useState } from "react";
import { ExternalLink, LoaderCircle } from "lucide-react";
import { api, resourcePath } from "@/lib/api";
import type { AuthResource } from "@/lib/types";
import { ErrorNotice } from "./status";
import { Button } from "./ui/button";

interface InstallationLink {
  url: string;
  appId: string;
  revision: number;
  incarnation: string;
}

export function AuthReauth({ profile }: { profile: AuthResource }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const request = useRef<AbortController | null>(null);
  const latestProfile = useRef(profile);
  latestProfile.current = profile;
  useEffect(() => () => request.current?.abort(), []);

  async function openGitHub() {
    if (busy) return;
    const controller = new AbortController();
    request.current = controller;
    setBusy(true);
    setError(null);
    try {
      const { data } = await api<InstallationLink>(
        `${resourcePath("github-auth-profiles", profile.key)}/installation-link`,
        { signal: controller.signal },
      );
      if (controller.signal.aborted) return;
      const current = latestProfile.current;
      if (
        data.incarnation !== current.incarnation ||
        data.revision !== current.activeRevision ||
        data.appId !== current.active?.app_id
      )
        throw new Error("This connection changed. Refresh it before re-authenticating.");
      if (
        !/^https:\/\/github\.com\/apps\/[a-z0-9][a-z0-9-]{0,99}\/installations\/new$/.test(data.url)
      )
        throw new Error("GitHub returned an invalid installation link.");
      window.location.assign(data.url);
    } catch (thrown) {
      if (!controller.signal.aborted) setError(thrown);
    } finally {
      if (!controller.signal.aborted) setBusy(false);
    }
  }

  return (
    <>
      <Button size="sm" variant="outline" disabled={busy} onClick={() => void openGitHub()}>
        {busy ? <LoaderCircle className="animate-spin" /> : <ExternalLink />}
        {busy ? "Re-auth…" : "Re-auth"}
      </Button>
      {error !== null && (
        <div className="basis-full">
          <ErrorNotice error={error} retry={() => void openGitHub()} />
        </div>
      )}
    </>
  );
}
