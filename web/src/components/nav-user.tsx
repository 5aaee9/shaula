import { MoreVertical, RefreshCw } from "lucide-react";
import type { Session } from "@/lib/types";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from "@/components/ui/sidebar";

export function NavUser({ session, onRefresh }: { session?: Session; onRefresh: () => void }) {
  const { isMobile } = useSidebar();
  const name = session?.name || "Unauthenticated";
  const role = session
    ? session.scopes.some((scope) => /\.(write|retire)$/.test(scope))
      ? "Operator"
      : "Viewer"
    : "No session";
  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <SidebarMenuButton
              size="lg"
              aria-label="Session menu"
              className="data-[state=open]:bg-sidebar-accent data-[state=open]:text-sidebar-accent-foreground"
            >
              <Avatar className="size-8 rounded-lg">
                <AvatarFallback className="rounded-lg">
                  {name.charAt(0).toUpperCase()}
                </AvatarFallback>
              </Avatar>
              <div className="grid min-w-0 flex-1 text-left text-sm leading-tight">
                <span className="truncate font-medium">{name}</span>
                <span className="truncate text-xs text-muted-foreground">{role}</span>
              </div>
              <MoreVertical className="ml-auto size-4" />
            </SidebarMenuButton>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            className="w-(--radix-dropdown-menu-trigger-width) min-w-56 max-w-[calc(100vw-2rem)] rounded-lg"
            side={isMobile ? "top" : "right"}
            align="end"
            sideOffset={4}
          >
            <DropdownMenuLabel className="truncate">{name}</DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuLabel className="text-xs text-muted-foreground">
              Permissions
            </DropdownMenuLabel>
            <div className="max-h-48 overflow-y-auto px-2 pb-2 text-xs text-muted-foreground">
              {session?.scopes.length
                ? session.scopes.map((scope) => (
                    <div key={scope} className="py-1 break-all">
                      {scope}
                    </div>
                  ))
                : "None"}
            </div>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={onRefresh}>
              <RefreshCw />
              Refresh session
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}
