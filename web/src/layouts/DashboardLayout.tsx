import { NavLink, Outlet } from 'react-router';
import {
  LayoutDashboard,
  Briefcase,
  FileText,
  MessageSquare,
  Search,
  Bell,
  Clock,
} from 'lucide-react';
import { cn } from '@/lib/utils';
import { Separator } from '@/components/ui/separator';

const navItems = [
  { to: '/', icon: LayoutDashboard, label: 'Dashboard' },
  { to: '/applications', icon: Briefcase, label: 'Applications' },
  { to: '/resumes', icon: FileText, label: 'Resumes' },
  { to: '/interviews', icon: MessageSquare, label: 'Interviews' },
  { to: '/discovery', icon: Search, label: 'Job Discovery' },
  { to: '/notifications', icon: Bell, label: 'Notifications' },
  { to: '/scheduler', icon: Clock, label: 'Scheduler' },
];

export default function DashboardLayout() {
  return (
    <div className="flex h-screen">
      {/* Sidebar */}
      <aside className="w-64 border-r bg-card flex flex-col">
        <div className="p-6">
          <h1 className="text-xl font-bold">Job Platform</h1>
        </div>
        <Separator />
        <nav className="flex-1 p-4 space-y-1">
          {navItems.map((item) => (
            <NavLink
              key={item.to}
              to={item.to}
              end={item.to === '/'}
              className={({ isActive }) =>
                cn(
                  'flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors',
                  isActive
                    ? 'bg-accent text-accent-foreground'
                    : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground'
                )
              }
            >
              <item.icon className="h-4 w-4" />
              {item.label}
            </NavLink>
          ))}
        </nav>
      </aside>

      {/* Main content */}
      <main className="flex-1 overflow-auto">
        <div className="p-8">
          <Outlet />
        </div>
      </main>
    </div>
  );
}
