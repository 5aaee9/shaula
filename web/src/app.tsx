import { useSyncExternalStore, type CSSProperties } from "react";
import {
  authenticationExpired,
  authenticatedFetch,
  loginUrl,
  subscribeAuthentication,
} from "@/lib/authentication";
import { useQuery } from "@tanstack/react-query";
import { Navigate, NavLink, Route, Routes, useLocation } from "react-router-dom";
import { useSession } from "@/lib/queries";
import { Button } from "@/components/ui/button";
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar";
import { AppSidebar, navigation } from "@/components/app-sidebar";
import { SiteHeader } from "@/components/site-header";
import { ErrorNotice, Loading } from "@/components/status";
import { FleetsPage } from "@/pages/fleets";
import { FleetDetail } from "@/pages/fleet-detail";
import { TemplatesPage } from "@/pages/templates";
import { TemplatePublishPage } from "@/pages/template-publish";
import { AuthPage } from "@/pages/auth";
import { ChangesPage } from "@/pages/changes";

export function App() {
  const expired = useSyncExternalStore(subscribeAuthentication, authenticationExpired);
  const session = useSession();
  const location = useLocation();
  const health = useQuery({
    queryKey: ["health"],
    queryFn: async ({ signal }) => {
      const response = await authenticatedFetch("/readyz", { signal });
      if (![200, 503].includes(response.status)) throw new Error("Health unavailable");
      return (await response.json()) as { ready: boolean };
    },
    refetchInterval: 10_000,
    enabled: !expired,
  });
  const current =
    navigation.find((item) => location.pathname.startsWith(item.url))?.title || "Control plane";
  const scopes = session.data?.data.scopes || [];
  const healthLabel = health.error
    ? "Offline"
    : health.isPending
      ? "Connecting"
      : health.data?.ready
        ? "Ready"
        : "Not ready";
  if (expired)
    return (
      <main className="flex min-h-svh flex-col items-center justify-center gap-4 p-6">
        <h1 className="text-xl font-semibold">Sign in to Shaula</h1>
        <Button asChild>
          <a href={loginUrl()}>Sign in</a>
        </Button>
      </main>
    );
  return (
    <SidebarProvider
      style={
        {
          "--sidebar-width": "calc(var(--spacing) * 64)",
          "--header-height": "calc(var(--spacing) * 12)",
        } as CSSProperties
      }
    >
      <AppSidebar
        variant="inset"
        session={session.data?.data}
        onRefreshSession={() => void session.refetch()}
      />
      <SidebarInset className="min-w-0">
        <SiteHeader title={current} health={healthLabel} />
        <div className="flex min-w-0 flex-1 flex-col">
          <div
            id="main-content"
            className="@container/main min-w-0 flex-1 px-4 py-4 md:py-6 lg:px-6"
          >
            {session.isPending ? (
              <Loading />
            ) : session.error ? (
              <>
                <h1>Authentication required</h1>
                <ErrorNotice error={session.error} retry={() => void session.refetch()} />
              </>
            ) : (
              <Routes>
                <Route path="/" element={<Navigate to="/fleets" replace />} />
                <Route path="/fleets" element={<FleetsPage scopes={scopes} />} />
                <Route path="/fleets/:key" element={<FleetDetail scopes={scopes} />} />
                <Route path="/templates" element={<TemplatesPage scopes={scopes} />} />
                <Route
                  path="/templates/new"
                  element={<TemplatePublishPage key="new" scopes={scopes} />}
                />
                <Route
                  path="/templates/:key/revisions/new"
                  element={<TemplatePublishPage key="revision" scopes={scopes} />}
                />
                <Route path="/auth" element={<AuthPage scopes={scopes} />} />
                <Route path="/changes" element={<ChangesPage scopes={scopes} />} />
                <Route
                  path="*"
                  element={
                    <>
                      <h1>Page not found</h1>
                      <Button asChild variant="outline">
                        <NavLink to="/fleets">Back to fleets</NavLink>
                      </Button>
                    </>
                  }
                />
              </Routes>
            )}
          </div>
        </div>
      </SidebarInset>
    </SidebarProvider>
  );
}
