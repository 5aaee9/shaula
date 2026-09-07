import type { ComponentProps } from "react";
import { Boxes, GitPullRequest, KeyRound, Layers3 } from "lucide-react";
import { Link } from "react-router-dom";
import type { Session } from "@/lib/types";
import { NavMain } from "@/components/nav-main";
import { NavUser } from "@/components/nav-user";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from "@/components/ui/sidebar";

export const navigation = [
  { url: "/fleets", title: "Fleets", icon: Boxes },
  { url: "/templates", title: "Templates", icon: Layers3 },
  { url: "/auth", title: "GitHub authentication", icon: KeyRound },
  { url: "/changes", title: "Changes", icon: GitPullRequest },
];

// Adapted from shadcn/ui dashboard-01; menus are bound to Shaula routes and session.
export function AppSidebar({
  session,
  onRefreshSession,
  ...props
}: ComponentProps<typeof Sidebar> & {
  session?: Session;
  onRefreshSession: () => void;
}) {
  const { setOpenMobile } = useSidebar();
  return (
    <Sidebar collapsible="offcanvas" {...props}>
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton asChild className="data-[slot=sidebar-menu-button]:p-1.5!">
              <Link to="/fleets" onClick={() => setOpenMobile(false)}>
                <img
                  src="/shaula-mark.png"
                  width={24}
                  height={24}
                  alt=""
                  className="size-6 grayscale"
                />
                <span className="text-base font-semibold">Shaula</span>
              </Link>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>
      <SidebarContent>
        <NavMain items={navigation} />
      </SidebarContent>
      <SidebarFooter>
        <NavUser session={session} onRefresh={onRefreshSession} />
      </SidebarFooter>
    </Sidebar>
  );
}
